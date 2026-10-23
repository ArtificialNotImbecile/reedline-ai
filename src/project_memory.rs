//! Explicit, local-only project memory. Nothing is discovered or uploaded.
//!
//! Only `confirmed` facts participate in command resolution. Suggestions are
//! inert notes, never executable recipes or automatically promoted facts.
//! JSON is user editable; loading never rewrites or changes permissions.
//!
//! Writes use a same-directory, create-new private temporary file, sync, and
//! atomic replacement. Unix ownership, modes, link counts, and file identities
//! are checked. The standard library has no portable directory-handle-relative
//! rename or compare-and-swap: a malicious process running as the same user can
//! still race these checks. On non-Unix platforms, ACLs are not verified and
//! identity checks are best effort. Use a directory controlled by the user.

use crate::{context::unsafe_char, shell::ShellKind};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const MEMORY_VERSION: u32 = 1;
pub const MAX_MEMORY_BYTES: usize = 256 * 1024;
const MAX_NAME_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 8 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMemory {
    /// Required in persisted JSON; unknown versions are rejected.
    pub version: u32,
    #[serde(default, deserialize_with = "unique_map")]
    pub confirmed: BTreeMap<String, Project>,
    #[serde(default)]
    pub suggestions: Vec<MemorySuggestion>,
}

impl Default for ProjectMemory {
    fn default() -> Self {
        Self {
            version: MEMORY_VERSION,
            confirmed: BTreeMap::new(),
            suggestions: Vec::new(),
        }
    }
}

// Deliberately do not print paths, aliases, commands, or private notes.
impl fmt::Debug for ProjectMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectMemory")
            .field("version", &self.version)
            .field("confirmed_projects", &self.confirmed.len())
            .field("suggestions", &self.suggestions.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Absolute in global/local/explicit memory; root-relative in project memory.
    /// Never expanded, probed, or inferred.
    pub path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Descriptive metadata only. This never activates an environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, deserialize_with = "unique_map")]
    pub workflows: BTreeMap<String, Workflow>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    /// Full matching phrases, such as "start CCTV" or "启动监控".
    #[serde(default)]
    pub aliases: Vec<String>,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WorkflowStep {
    /// Uses the containing project's explicit path.
    ChangeDirectory,
    /// Typed arguments are quoted for the selected shell, not concatenated.
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// A whole, explicitly shell-specific recipe. Never translated or mixed
    /// with typed steps. Its author is responsible for its shell semantics.
    Shell { shell: ShellKind, command: String },
}

// Serde's internally tagged unit variants otherwise ignore extra fields even
// with deny_unknown_fields. Decode through an empty *struct* variant instead.
impl<'de> Deserialize<'de> for WorkflowStep {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum StrictStep {
            ChangeDirectory {},
            Command {
                program: String,
                #[serde(default)]
                args: Vec<String>,
            },
            Shell {
                shell: ShellKind,
                command: String,
            },
        }
        Ok(match StrictStep::deserialize(deserializer)? {
            StrictStep::ChangeDirectory {} => Self::ChangeDirectory,
            StrictStep::Command { program, args } => Self::Command { program, args },
            StrictStep::Shell { shell, command } => Self::Shell { shell, command },
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySuggestion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub note: String,
}

/// The shared resolver normalization. Does not alter executable strings.
pub fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl ProjectMemory {
    /// Read only the caller-selected JSON file, bounded before and during read.
    /// Filesystem and parse errors never include the path or JSON contents.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let memory = Self::load_unresolved(path.as_ref())?;
        memory.require_absolute_paths()?;
        Ok(memory)
    }

    /// Read an editable document without resolving project-relative paths.
    /// For management only: this does not make the document executable or
    /// suitable for workflow resolution. Use load/load_layers for that.
    pub fn load_document(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::load_unresolved(path.as_ref())
    }

    /// Merge only explicitly selected files, lowest precedence first. Higher
    /// layers replace an entire same-named project, never individual fields.
    /// Suggestions remain inert and are appended in layer order. No file is
    /// discovered automatically and no endpoint or upload policy is accepted.
    pub fn load_layers(
        global: Option<&Path>,
        project: Option<(&Path, &Path)>,
        local: Option<&Path>,
        explicit: Option<&Path>,
    ) -> io::Result<Self> {
        let mut result = Self::default();
        if let Some(path) = global {
            result.merge_layer(Self::load(path)?);
        }
        if let Some((path, root)) = project {
            let mut layer = Self::load_unresolved(path)?;
            layer.resolve_relative_paths(root)?;
            result.merge_layer(layer);
        }
        for path in [local, explicit].into_iter().flatten() {
            result.merge_layer(Self::load(path)?);
        }
        result.validate()?;
        result.require_absolute_paths()?;
        Ok(result)
    }

    /// Resolve a project-scoped layer under one explicitly supplied existing
    /// root. Absolute project-layer paths are forbidden. Normalization is
    /// lexical: target directories are not inspected, and a target symlink may
    /// therefore lead elsewhere. The caller must trust the selected project.
    pub fn resolve_relative_paths(&mut self, root: impl AsRef<Path>) -> io::Result<()> {
        self.validate()?;
        let root = fs::canonicalize(root.as_ref())
            .map_err(|_| failure("explicit project root does not exist or is inaccessible"))?;
        if !root.is_dir() {
            return Err(failure("explicit project root must be a directory"));
        }
        let mut next = self.clone();
        for project in next.confirmed.values_mut() {
            let parts = relative_parts(&project.path).ok_or_else(|| {
                failure("project-layer paths must be relative and remain inside the explicit root")
            })?;
            let mut resolved = root.clone();
            for part in parts {
                resolved.push(part);
            }
            let path = resolved
                .to_str()
                .ok_or_else(|| failure("explicit project root must be valid Unicode"))?;
            // Windows canonicalize uses extended-length syntax. Normalize the
            // canonical result back to an ordinary drive/UNC display path.
            project.path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{unc}")
            } else {
                path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
            };
        }
        next.validate()?;
        next.require_absolute_paths()?;
        *self = next;
        Ok(())
    }

    fn require_absolute_paths(&self) -> io::Result<()> {
        if self
            .confirmed
            .values()
            .any(|project| !is_explicit_absolute_path(&project.path))
        {
            return Err(failure(
                "relative project paths require an explicitly selected project layer and root",
            ));
        }
        Ok(())
    }

    fn merge_layer(&mut self, layer: Self) {
        self.confirmed.extend(layer.confirmed);
        self.suggestions.extend(layer.suggestions);
    }

    fn load_unresolved(path: &Path) -> io::Result<Self> {
        let parents = inspect_parents(path)?;
        let before = regular_metadata(path)?;
        let file = File::open(path).map_err(|_| failure("could not open project memory"))?;
        let opened = file
            .metadata()
            .map_err(|_| failure("could not inspect project memory"))?;
        if !same_file(&before, &opened) || !opened.is_file() {
            return Err(failure("project memory changed while opening"));
        }
        verify_parents(&parents)?;
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_MEMORY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure("could not read project memory"))?;
        if bytes.len() > MAX_MEMORY_BYTES {
            return Err(failure("project memory exceeds the 256 KiB limit"));
        }
        let after = file
            .metadata()
            .map_err(|_| failure("could not inspect project memory after reading"))?;
        if !unchanged_file(&before, &after) || !same_file(&after, &regular_metadata(path)?) {
            return Err(failure("project memory changed while reading"));
        }
        verify_parents(&parents)?;
        let memory: Self = serde_json::from_slice(&bytes)
            .map_err(|_| failure("project memory JSON does not match the supported schema"))?;
        memory.validate()?;
        Ok(memory)
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.version != MEMORY_VERSION {
            return Err(failure(
                "unsupported project memory version; expected version 1",
            ));
        }
        let mut project_names = BTreeSet::new();
        let mut phrases = BTreeSet::new();
        for (name, project) in &self.confirmed {
            let names = names_for(name, &project.aliases)?;
            for name in &names {
                if !project_names.insert(name.clone()) {
                    return Err(failure("project names or aliases are ambiguous"));
                }
            }
            validate_text(&project.path, false)?;
            if !is_explicit_absolute_path(&project.path) && relative_parts(&project.path).is_none()
            {
                return Err(failure(
                    "project paths must be explicit absolute paths or safe root-relative paths",
                ));
            }
            if let Some(environment) = &project.environment {
                validate_text(environment, false)?;
            }
            let mut workflow_names = BTreeSet::new();
            for (workflow_name, workflow) in &project.workflows {
                validate_name(workflow_name)?;
                let action = normalize_name(workflow_name);
                if !workflow_names.insert(action.clone()) {
                    return Err(failure("workflow names are ambiguous"));
                }
                for name in &names {
                    if !phrases.insert(format!("{action} {name}")) {
                        return Err(failure("generated workflow matching phrases are ambiguous"));
                    }
                }
                // Workflow aliases are whole natural-language phrases, not
                // verbs to be combined with project names a second time.
                for alias in &workflow.aliases {
                    validate_name(alias)?;
                    if !phrases.insert(normalize_name(alias)) {
                        return Err(failure(
                            "workflow matching phrases or aliases are ambiguous",
                        ));
                    }
                }
                validate_workflow(workflow)?;
            }
        }

        for suggestion in &self.suggestions {
            if let Some(project) = &suggestion.project {
                validate_name(project)?;
            }
            validate_text(&suggestion.note, false)?;
        }
        let encoded = serde_json::to_vec_pretty(self)
            .map_err(|_| failure("could not encode project memory"))?;
        if encoded.len() + 1 > MAX_MEMORY_BYTES {
            return Err(failure("project memory exceeds the 256 KiB limit"));
        }
        Ok(())
    }

    /// Persist explicitly, without creating parent directories or following
    /// symlinks. Existing Unix files must already be private (mode 0600).
    /// A missing destination is linked without clobbering a concurrent creator.
    /// Existing destinations are identity-checked before atomic replacement.
    pub fn write_atomic(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let parents = inspect_parents(path)?;
        check_parent_modes(&parents)?;
        let original = optional_regular_metadata(path)?;
        if let Some(metadata) = &original {
            check_private_file(metadata)?;
        }
        let parent = parent_path(path);
        let (mut file, mut temporary) = create_temporary(parent)?;
        let own_metadata = file
            .metadata()
            .map_err(|_| failure("could not inspect temporary project memory"))?;
        check_private_file(&own_metadata)?;
        check_owners(&parents, original.as_ref(), &own_metadata)?;
        verify_parents(&parents)?;
        let mut encoded = serde_json::to_vec_pretty(self)
            .map_err(|_| failure("could not encode project memory"))?;
        encoded.push(b'\n');
        file.write_all(&encoded)
            .and_then(|()| file.sync_all())
            .map_err(|_| failure("could not write and sync temporary project memory"))?;
        drop(file);
        verify_parents(&parents)?;
        let current = optional_regular_metadata(path)?;
        match (&original, &current) {
            (Some(before), Some(now)) if unchanged_file(before, now) => {
                check_private_file(now)?;
                check_owners(&parents, Some(now), &own_metadata)?;
                fs::rename(&temporary.path, path)
                    .map_err(|_| failure("could not replace project memory atomically"))?;
                temporary.active = false;
            }
            (None, None) => {
                // Hard-link publication is atomic and will not overwrite a
                // destination created after the existence check above.
                fs::hard_link(&temporary.path, path).map_err(|_| {
                    failure("could not create project memory without replacing an existing file")
                })?;
                fs::remove_file(&temporary.path).map_err(|_| {
                    failure("project memory was created but temporary cleanup failed")
                })?;
                temporary.active = false;
            }
            _ => {
                return Err(failure(
                    "project memory changed before replacement; nothing was replaced",
                ));
            }
        }
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| {
                failure(
                    "project memory was replaced but directory durability could not be confirmed",
                )
            })?;
        Ok(())
    }

    /// Mutations are transactional: validation failure leaves memory unchanged.
    /// These helpers modify only this value. The caller must explicitly save it.
    pub fn add_project(&mut self, name: impl Into<String>, project: Project) -> io::Result<()> {
        let name = name.into();
        if self.confirmed.contains_key(&name) {
            return Err(failure("project already exists; use an explicit update"));
        }
        self.edit(|next| {
            next.confirmed.insert(name, project);
        })
    }

    pub fn update_project(&mut self, name: &str, project: Project) -> io::Result<()> {
        if !self.confirmed.contains_key(name) {
            return Err(failure("confirmed project does not exist"));
        }
        self.edit(|next| {
            next.confirmed.insert(name.to_owned(), project);
        })
    }

    pub fn remove_project(&mut self, name: &str) -> io::Result<Project> {
        let project = self
            .confirmed
            .get(name)
            .ok_or_else(|| failure("confirmed project does not exist"))?
            .clone();
        self.edit(|next| {
            next.confirmed.remove(name);
        })?;
        Ok(project)
    }

    pub fn add_workflow(
        &mut self,
        project: &str,
        name: impl Into<String>,
        workflow: Workflow,
    ) -> io::Result<()> {
        let name = name.into();
        let entry = self
            .confirmed
            .get(project)
            .ok_or_else(|| failure("confirmed project does not exist"))?;
        if entry.workflows.contains_key(&name) {
            return Err(failure("workflow already exists; use an explicit update"));
        }
        self.edit(|next| {
            next.confirmed
                .get_mut(project)
                .expect("checked project")
                .workflows
                .insert(name, workflow);
        })
    }

    pub fn update_workflow(
        &mut self,
        project: &str,
        name: &str,
        workflow: Workflow,
    ) -> io::Result<()> {
        self.get_workflow(project, name)?;
        self.edit(|next| {
            next.confirmed
                .get_mut(project)
                .expect("checked project")
                .workflows
                .insert(name.to_owned(), workflow);
        })
    }

    pub fn remove_workflow(&mut self, project: &str, name: &str) -> io::Result<Workflow> {
        let workflow = self.get_workflow(project, name)?.clone();
        self.edit(|next| {
            next.confirmed
                .get_mut(project)
                .expect("checked project")
                .workflows
                .remove(name);
        })?;
        Ok(workflow)
    }

    pub fn add_suggestion(&mut self, suggestion: MemorySuggestion) -> io::Result<()> {
        self.edit(|next| next.suggestions.push(suggestion))
    }

    pub fn update_suggestion(
        &mut self,
        index: usize,
        suggestion: MemorySuggestion,
    ) -> io::Result<()> {
        if index >= self.suggestions.len() {
            return Err(failure("suggestion does not exist"));
        }
        self.edit(|next| next.suggestions[index] = suggestion)
    }

    pub fn remove_suggestion(&mut self, index: usize) -> io::Result<MemorySuggestion> {
        let suggestion = self
            .suggestions
            .get(index)
            .ok_or_else(|| failure("suggestion does not exist"))?
            .clone();
        self.edit(|next| {
            next.suggestions.remove(index);
        })?;
        Ok(suggestion)
    }

    fn get_workflow(&self, project: &str, name: &str) -> io::Result<&Workflow> {
        self.confirmed
            .get(project)
            .and_then(|entry| entry.workflows.get(name))
            .ok_or_else(|| failure("confirmed workflow does not exist"))
    }

    fn edit(&mut self, edit: impl FnOnce(&mut Self)) -> io::Result<()> {
        let mut next = self.clone();
        edit(&mut next);
        next.validate()?;
        *self = next;
        Ok(())
    }
}

fn validate_workflow(workflow: &Workflow) -> io::Result<()> {
    if workflow.steps.is_empty() {
        return Err(failure("workflows must contain at least one step"));
    }
    for step in &workflow.steps {
        match step {
            WorkflowStep::ChangeDirectory => {}
            WorkflowStep::Command { program, args } => {
                // A command name only, never flags, paths, assignments, or
                // shell syntax. Shell lowering also rejects reserved words.
                if program.len() > MAX_NAME_BYTES || !crate::shell::valid_program(program) {
                    return Err(failure(
                        "typed command programs must be plain executable names",
                    ));
                }
                for arg in args {
                    validate_text(arg, true)?;
                }
            }
            WorkflowStep::Shell { command, .. } => {
                if workflow.steps.len() != 1 {
                    return Err(failure(
                        "raw shell workflows must use one complete explicit shell recipe, not mixed or multiple steps",
                    ));
                }
                validate_text(command, false)?;
            }
        }
    }
    Ok(())
}

fn names_for(name: &str, aliases: &[String]) -> io::Result<Vec<String>> {
    let mut names = BTreeSet::new();
    for name in std::iter::once(name).chain(aliases.iter().map(String::as_str)) {
        validate_name(name)?;
        if !names.insert(normalize_name(name)) {
            return Err(failure("a name or alias is repeated after normalization"));
        }
    }
    Ok(names.into_iter().collect())
}

fn validate_name(name: &str) -> io::Result<()> {
    validate_text(name, false)?;
    if name.len() > MAX_NAME_BYTES || name != name.trim() {
        return Err(failure(
            "names must be at most 128 bytes with no surrounding whitespace",
        ));
    }
    Ok(())
}

fn validate_text(text: &str, allow_empty: bool) -> io::Result<()> {
    if (!allow_empty && text.trim().is_empty())
        || text.len() > MAX_TEXT_BYTES
        || text.chars().any(unsafe_char)
    {
        return Err(failure(
            "memory text must be bounded single-line text without terminal controls",
        ));
    }
    Ok(())
}

/// Syntax only; never touches the project path on disk.
pub fn is_explicit_absolute_path(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return true;
    }
    if let Some(unc) = path.strip_prefix("\\\\") {
        // Reject device/extended namespaces, incomplete shares, and empty
        // host/share names. These are not ordinary explicit project paths.
        let mut parts = unc.split('\\');
        let host = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        return !host.is_empty() && !matches!(host, "." | "?") && !share.is_empty();
    }
    false
}

/// Normalize cross-platform relative syntax without touching target paths.
fn relative_parts(path: &str) -> Option<Vec<&str>> {
    if path.is_empty()
        || is_explicit_absolute_path(path)
        || path.starts_with(['\\', '~', '$', '%'])
        || path.contains(':')
    {
        return None;
    }
    let mut normalized = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                normalized.pop()?;
            }
            value => normalized.push(value),
        }
    }
    Some(normalized)
}

fn failure(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct UniqueMap<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> de::Visitor<'de> for UniqueMap<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object without duplicate keys")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut output = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, V>()? {
                if output.insert(key, value).is_some() {
                    return Err(de::Error::custom("duplicate memory key"));
                }
            }
            Ok(output)
        }
    }
    deserializer.deserialize_map(UniqueMap(std::marker::PhantomData))
}

fn parent_path(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

type Parents = Vec<(PathBuf, Metadata)>;
fn inspect_parents(path: &Path) -> io::Result<Parents> {
    if path.file_name().is_none()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(failure(
            "memory file must have an explicit filename without parent traversal",
        ));
    }
    let parent = parent_path(path);
    let mut paths: Vec<PathBuf> = parent
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .collect();
    if !parent.is_absolute() && !paths.iter().any(|p| p == Path::new(".")) {
        paths.push(PathBuf::from("."));
    }
    paths.reverse();
    paths
        .into_iter()
        .map(|path| {
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| failure("memory parent directory is missing or inaccessible"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(failure(
                    "memory parent directories must be real directories, not symlinks",
                ));
            }
            Ok((path, metadata))
        })
        .collect()
}

fn verify_parents(parents: &Parents) -> io::Result<()> {
    for (path, before) in parents {
        let now =
            fs::symlink_metadata(path).map_err(|_| failure("memory parent directory changed"))?;
        if !now.is_dir() || now.file_type().is_symlink() || !same_file(before, &now) {
            return Err(failure("memory parent directory changed"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.uid() != now.uid() || before.mode() != now.mode() {
                return Err(failure("memory parent ownership or permissions changed"));
            }
        }
    }
    Ok(())
}

fn regular_metadata(path: &Path) -> io::Result<Metadata> {
    optional_regular_metadata(path)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "project memory file does not exist",
        )
    })
}

fn optional_regular_metadata(path: &Path) -> io::Result<Option<Metadata>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failure("could not inspect project memory")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(failure(
            "project memory must be a regular file, not a symlink",
        ));
    }
    if metadata.len() > MAX_MEMORY_BYTES as u64 {
        return Err(failure("project memory exceeds the 256 KiB limit"));
    }
    Ok(Some(metadata))
}

fn check_private_file(metadata: &Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o7777 != 0o600 || metadata.nlink() != 1 {
            return Err(failure(
                "writable memory files must have Unix mode 0600 and exactly one hard link",
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

fn check_parent_modes(parents: &Parents) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        for (index, (_, metadata)) in parents.iter().enumerate() {
            // A sticky ancestor such as /tmp is allowed, but never as the
            // immediate storage directory. That directory must be controlled.
            let sticky_ancestor = index + 1 < parents.len() && metadata.mode() & 0o1000 != 0;
            if metadata.mode() & 0o022 != 0 && !sticky_ancestor {
                return Err(failure(
                    "memory storage directory must not be group or world writable",
                ));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = parents;
    Ok(())
}

fn check_owners(
    parents: &Parents,
    existing: Option<&Metadata>,
    temporary: &Metadata,
) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // A newly created file is owned by this effective user. No OS account
        // lookup, environment-variable trust, subprocess, or /proc read.
        let owner = temporary.uid();
        if existing.is_some_and(|metadata| metadata.uid() != owner)
            || parents
                .iter()
                .any(|(_, metadata)| metadata.uid() != owner && metadata.uid() != 0)
            || parents
                .last()
                .is_some_and(|(_, metadata)| metadata.uid() != owner)
        {
            return Err(failure(
                "memory file and storage directory must be owned by the current user",
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = (parents, existing, temporary);
    Ok(())
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        left.is_file() == right.is_file()
            && left.is_dir() == right.is_dir()
            && left.len() == right.len()
            && left.created().ok() == right.created().ok()
    }
}

fn unchanged_file(before: &Metadata, now: &Metadata) -> bool {
    if !same_file(before, now)
        || before.len() != now.len()
        || before.modified().ok() != now.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.ctime() == now.ctime()
            && before.ctime_nsec() == now.ctime_nsec()
            && before.mode() == now.mode()
            && before.uid() == now.uid()
            && before.nlink() == now.nlink()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

struct TemporaryFile {
    path: PathBuf,
    metadata: Metadata,
    active: bool,
}
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.active
            && fs::symlink_metadata(&self.path).is_ok_and(|now| same_file(&self.metadata, &now))
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_temporary(parent: &Path) -> io::Result<(File, TemporaryFile)> {
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".project-memory-{}-{sequence}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                let metadata = file
                    .metadata()
                    .map_err(|_| failure("could not inspect temporary project memory"))?;
                return Ok((
                    file,
                    TemporaryFile {
                        path,
                        metadata,
                        active: true,
                    },
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(failure("could not create private temporary project memory")),
        }
    }
    Err(failure(
        "could not reserve a temporary project memory filename",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ProjectMemory {
        serde_json::from_str(include_str!("../examples/project-memory.synthetic.json")).unwrap()
    }
    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "reedline-memory-synthetic-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    #[test]
    fn synthetic_fixture_round_trips_and_suggestions_remain_inert() {
        let memory = fixture();
        memory.validate().unwrap();
        let encoded = serde_json::to_string(&memory).unwrap();
        let loaded: ProjectMemory = serde_json::from_str(&encoded).unwrap();
        assert_eq!(memory, loaded);
        assert_eq!(memory.confirmed.len(), 2);
        assert_eq!(memory.suggestions.len(), 1);
        assert!(!format!("{memory:?}").contains("synthetic"));
    }

    #[test]
    fn schema_is_strict_and_duplicate_keys_are_rejected() {
        for json in [
            r#"{"confirmed":{}}"#,
            r#"{"version":1,"unknown":"private"}"#,
            r#"{"version":1,"confirmed":{"x":{"path":"/synthetic","extra":true}}}"#,
            r#"{"version":1,"confirmed":{"x":{"path":"/a"},"x":{"path":"/b"}}}"#,
            r#"{"version":1,"confirmed":{"x":{"path":"/a","workflows":{"run":{"steps":[{"action":"change_directory","path":"/evil"}]}}}}}"#,
            r#"{"version":1,"confirmed":{"x":{"path":"/a","workflows":{"run":{"steps":[{"action":"command","program":"echo","extra":1}]}}}}}"#,
        ] {
            assert!(serde_json::from_str::<ProjectMemory>(json).is_err());
        }
        let memory = ProjectMemory {
            version: 2,
            ..ProjectMemory::default()
        };
        assert!(memory.validate().is_err());
        let minimal: ProjectMemory = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(minimal, ProjectMemory::default());
    }

    #[test]
    fn paths_are_syntax_only_and_controls_are_rejected() {
        for path in [
            "/synthetic/not-real",
            r"C:\synthetic\repo",
            "Z:/synthetic/repo",
            r"\\synthetic-server\share\repo",
        ] {
            assert!(is_explicit_absolute_path(path));
        }
        for path in [
            "relative",
            "~/repo",
            "$HOME/repo",
            "C:repo",
            r"\repo",
            r"\\server",
            r"\\?\C:\repo",
        ] {
            assert!(!is_explicit_absolute_path(path));
        }
        let mut memory = fixture();
        memory.confirmed.get_mut("CCTV").unwrap().path = "/synthetic/\x1b[31m".into();
        assert!(memory.validate().is_err());
    }

    #[test]
    fn aliases_and_generated_phrases_must_be_unambiguous() {
        let mut memory = fixture();
        memory
            .confirmed
            .get_mut("CCTV")
            .unwrap()
            .aliases
            .push("cctv".into());
        assert!(memory.validate().is_err());
        let mut memory = fixture();
        memory
            .confirmed
            .get_mut("Website")
            .unwrap()
            .aliases
            .push("CAMERAS".into());
        assert!(memory.validate().is_err());
        let mut memory = ProjectMemory::default();
        let workflow = Workflow {
            aliases: vec![],
            steps: vec![WorkflowStep::ChangeDirectory],
        };
        memory
            .add_project(
                "camera app",
                Project {
                    path: "/synthetic/a".into(),
                    aliases: vec![],
                    environment: None,
                    workflows: BTreeMap::from([("run".into(), workflow.clone())]),
                },
            )
            .unwrap();
        assert!(
            memory
                .add_project(
                    "app",
                    Project {
                        path: "/synthetic/b".into(),
                        aliases: vec![],
                        environment: None,
                        workflows: BTreeMap::from([("run camera".into(), workflow)])
                    }
                )
                .is_err()
        );
        assert_eq!(normalize_name("  Run   CAMERAS "), "run cameras");
        let mut memory = fixture();
        memory
            .confirmed
            .get_mut("Website")
            .unwrap()
            .workflows
            .get_mut("run")
            .unwrap()
            .aliases
            .push("start CCTV".into());
        assert!(memory.validate().is_err());
        let mut memory = fixture();
        memory
            .confirmed
            .get_mut("Website")
            .unwrap()
            .workflows
            .get_mut("run")
            .unwrap()
            .aliases
            .push("run CCTV".into());
        assert!(memory.validate().is_err());
    }

    #[test]
    fn empty_or_unsafe_or_mixed_workflows_are_rejected() {
        for steps in [
            vec![],
            vec![WorkflowStep::Command {
                program: "echo;touch".into(),
                args: vec![],
            }],
            vec![WorkflowStep::Command {
                program: "--help".into(),
                args: vec![],
            }],
            vec![WorkflowStep::Command {
                program: "echo".into(),
                args: vec!["bad\narg".into()],
            }],
            vec![WorkflowStep::Shell {
                shell: ShellKind::Bash,
                command: "".into(),
            }],
            vec![
                WorkflowStep::ChangeDirectory,
                WorkflowStep::Shell {
                    shell: ShellKind::Bash,
                    command: "echo synthetic".into(),
                },
            ],
        ] {
            assert!(
                validate_workflow(&Workflow {
                    aliases: vec![],
                    steps
                })
                .is_err()
            );
        }
        validate_workflow(&Workflow {
            aliases: vec![],
            steps: vec![WorkflowStep::Command {
                program: "synthetic-tool.exe".into(),
                args: vec!["".into(), "literal; argument".into()],
            }],
        })
        .unwrap();
    }

    #[test]
    fn edits_validate_before_mutating_and_do_not_promote_notes() {
        let mut memory = fixture();
        let before = memory.clone();
        let mut invalid = memory.confirmed["CCTV"].clone();
        invalid.path = "../escape".into();
        assert!(memory.update_project("CCTV", invalid).is_err());
        assert_eq!(memory, before);
        let removed = memory.remove_project("CCTV").unwrap();
        assert_eq!(memory.suggestions.len(), 1);
        memory.add_project("CCTV", removed).unwrap();
        let run = memory.remove_workflow("CCTV", "run").unwrap();
        memory.add_workflow("CCTV", "run", run.clone()).unwrap();
        memory.update_workflow("CCTV", "run", run).unwrap();
        let note = memory.remove_suggestion(0).unwrap();
        memory.add_suggestion(note).unwrap();
        assert_eq!(memory, before);
    }

    #[test]
    fn reads_are_bounded_and_errors_do_not_reveal_paths_or_json() {
        let dir = temporary_directory();
        let path = dir.join("private-synthetic-value.json");
        assert_eq!(
            ProjectMemory::load(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        fs::write(&path, b"{\"secret-private-synthetic-value\":true}").unwrap();
        let error = ProjectMemory::load(&path).unwrap_err().to_string();
        assert!(!error.contains("private-synthetic-value"));
        fs::write(&path, vec![b' '; MAX_MEMORY_BYTES + 1]).unwrap();
        assert!(
            ProjectMemory::load(&path)
                .unwrap_err()
                .to_string()
                .contains("256 KiB")
        );
        fs::write(&path, b"{\"version\":99}").unwrap();
        assert!(
            ProjectMemory::load(&path)
                .unwrap_err()
                .to_string()
                .contains("version")
        );
        assert!(ProjectMemory::load(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_are_explicit_atomic_and_leave_no_temporary_files() {
        let dir = temporary_directory();
        let path = dir.join("memory.json");
        let mut memory = fixture();
        memory.write_atomic(&path).unwrap();
        assert_eq!(ProjectMemory::load(&path).unwrap(), memory);
        memory.remove_project("Website").unwrap();
        memory.write_atomic(&path).unwrap();
        assert_eq!(ProjectMemory::load(&path).unwrap(), memory);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        assert!(
            memory
                .write_atomic(dir.join("missing").join("memory.json"))
                .is_err()
        );
        assert!(!dir.join("missing").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_layers_replace_whole_projects_in_documented_order() {
        let dir = temporary_directory();
        let root = dir.join("project-root");
        fs::create_dir(&root).unwrap();
        let global = dir.join("global.json");
        let project_file = dir.join("project.json");
        let local = dir.join("local.json");
        let explicit = dir.join("explicit.json");
        let mut layer = fixture();
        layer.write_atomic(&global).unwrap();
        layer.confirmed.remove("Website");
        layer.confirmed.get_mut("CCTV").unwrap().path = "sub/../src".into();
        layer.confirmed.get_mut("CCTV").unwrap().aliases.clear();
        layer.suggestions.clear();
        layer.write_atomic(&project_file).unwrap();
        assert!(ProjectMemory::load(&project_file).is_err());
        let mut editable = ProjectMemory::load_document(&project_file).unwrap();
        assert_eq!(editable.confirmed["CCTV"].path, "sub/../src");
        editable.confirmed.get_mut("CCTV").unwrap().path = "src".into();
        editable.write_atomic(&project_file).unwrap();
        assert_eq!(
            ProjectMemory::load_document(&project_file).unwrap(),
            editable
        );
        let merged =
            ProjectMemory::load_layers(Some(&global), Some((&project_file, &root)), None, None)
                .unwrap();
        let expected = fs::canonicalize(&root).unwrap().join("src");
        let expected = expected.to_str().unwrap();
        let expected = if let Some(unc) = expected.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            expected
                .strip_prefix(r"\\?\")
                .unwrap_or(expected)
                .to_owned()
        };
        assert_eq!(merged.confirmed["CCTV"].path, expected);
        assert!(merged.confirmed["CCTV"].aliases.is_empty());
        assert!(merged.confirmed.contains_key("Website"));
        assert_eq!(merged.suggestions.len(), 1);
        layer.confirmed.get_mut("CCTV").unwrap().path = "/synthetic/local".into();
        layer.write_atomic(&local).unwrap();
        let merged = ProjectMemory::load_layers(
            Some(&global),
            Some((&project_file, &root)),
            Some(&local),
            None,
        )
        .unwrap();
        assert_eq!(merged.confirmed["CCTV"].path, "/synthetic/local");
        layer.confirmed.get_mut("CCTV").unwrap().path = "/synthetic/explicit".into();
        layer.write_atomic(&explicit).unwrap();
        let merged = ProjectMemory::load_layers(
            Some(&global),
            Some((&project_file, &root)),
            Some(&local),
            Some(&explicit),
        )
        .unwrap();
        assert_eq!(merged.confirmed["CCTV"].path, "/synthetic/explicit");
        assert!(!root.join("src").exists()); // target path never created or probed
        assert_eq!(
            ProjectMemory::load_layers(None, None, None, None).unwrap(),
            ProjectMemory::default()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn project_layer_rejects_absolute_and_escaping_paths_and_requires_root() {
        let dir = temporary_directory();
        let mut memory = fixture();
        assert!(memory.resolve_relative_paths(&dir).is_err());
        memory.confirmed.remove("Website");
        for invalid in [
            "../outside",
            "sub/../../outside",
            r"..\outside",
            "C:relative",
        ] {
            memory.confirmed.get_mut("CCTV").unwrap().path = invalid.into();
            assert!(memory.resolve_relative_paths(&dir).is_err());
        }
        memory.confirmed.get_mut("CCTV").unwrap().path = ".".into();
        assert!(memory.resolve_relative_paths(dir.join("missing")).is_err());
        memory.resolve_relative_paths(&dir).unwrap();
        assert!(is_explicit_absolute_path(&memory.confirmed["CCTV"].path));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_identity_and_content_change_checks_reject_replacements() {
        let dir = temporary_directory();
        let path = dir.join("first.json");
        fs::write(&path, b"synthetic").unwrap();
        let before = fs::metadata(&path).unwrap();
        fs::write(&path, b"different synthetic content").unwrap();
        assert!(!unchanged_file(&before, &fs::metadata(&path).unwrap()));
        let replacement = dir.join("replacement.json");
        fs::write(&replacement, b"replacement synthetic document").unwrap();
        assert!(!same_file(&before, &fs::metadata(&replacement).unwrap()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_writes_require_private_files_and_safe_parents() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let dir = temporary_directory();
        let path = dir.join("memory.json");
        let memory = fixture();
        memory.write_atomic(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let before = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(memory.write_atomic(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("symlink.json");
        symlink(&path, &link).unwrap();
        assert!(ProjectMemory::load(&link).is_err());
        assert!(memory.write_atomic(&link).is_err());
        let hard = dir.join("hard.json");
        fs::hard_link(&path, &hard).unwrap();
        assert!(memory.write_atomic(&path).is_err());
        fs::remove_file(hard).unwrap();
        let real = dir.join("real");
        fs::create_dir(&real).unwrap();
        let alias = dir.join("alias");
        symlink(&real, &alias).unwrap();
        assert!(memory.write_atomic(alias.join("memory.json")).is_err());
        fs::set_permissions(&real, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(memory.write_atomic(real.join("memory.json")).is_err());
        assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
