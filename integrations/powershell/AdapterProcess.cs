// Trusted adapter transport only. Suggestions are never compiled or executed.
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace ReedlineAi.PowerShellAdapterV1
{
    public sealed class Reply
    {
        public string Status { get; set; }
        public string Output { get; set; }
    }

    public sealed class Job
    {
        private static int active;
        private readonly TaskCompletionSource<bool> cancelled =
            new TaskCompletionSource<bool>(TaskCreationOptions.None);
        public Task<Reply> Completion { get; private set; }

        private Job() { }

        public static Job Start(string executable, string[] arguments, string cwd,
                                string request, int timeoutMilliseconds)
        {
            var job = new Job();
            // Process launch, input, output, waiting, and cancellation all run off
            // PSReadLine's UI thread. No PowerShell runspace is used by this worker.
            // One worker for this loaded helper type, including module reloads.
            // Fail busy rather than queue more processes behind an old request.
            if (Interlocked.CompareExchange(ref active, 1, 0) != 0)
            {
                job.Completion = Task.FromResult(Result("busy"));
                return job;
            }
            job.Completion = Task.Run(async () =>
            {
                try
                {
                    return await job.RunAsync(executable, arguments, cwd, request,
                                              timeoutMilliseconds).ConfigureAwait(false);
                }
                finally { Interlocked.Exchange(ref active, 0); }
            });
            return job;
        }

        public void Cancel() { cancelled.TrySetResult(true); }

        private async Task<Reply> RunAsync(string executable, string[] arguments, string cwd,
                                            string request, int timeoutMilliseconds)
        {
            using (var deadline = new CancellationTokenSource())
            using (var process = new Process())
            {
                Task exchange = null;
                try
                {
                    var encoding = new UTF8Encoding(false, true);
                    var start = new ProcessStartInfo
                    {
                        FileName = executable,
                        WorkingDirectory = cwd,
                        UseShellExecute = false,
                        CreateNoWindow = true,
                        RedirectStandardInput = true,
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        StandardOutputEncoding = encoding,
                        StandardErrorEncoding = encoding
                    };
                    ConfigureArguments(start, arguments);
                    // StandardInputEncoding is absent on older .NET Framework.
                    var inputEncoding = typeof(ProcessStartInfo).GetProperty("StandardInputEncoding");
                    if (inputEncoding != null) inputEncoding.SetValue(start, encoding, null);
                    process.StartInfo = start;
                    if (cancelled.Task.IsCompleted) return Result("cancelled");
                    var timeout = Task.Delay(timeoutMilliseconds, deadline.Token);
                    if (!process.Start()) return Result("failed");
                    var output = ReadBoundedAsync(process.StandardOutput, true);
                    var error = ReadBoundedAsync(process.StandardError, false);
                    var input = WriteInputAsync(process.StandardInput.BaseStream, request);
                    var exited = WaitForExitAsync(process);
                    exchange = Task.WhenAll(input, output, error, exited);
                    var winner = await Task.WhenAny(exchange, timeout, cancelled.Task)
                                           .ConfigureAwait(false);
                    if (winner != exchange)
                    {
                        Kill(process);
                        // Never hold the editor, nor wait indefinitely for inherited
                        // handles or a descendant that cannot be terminated.
                        await Task.WhenAny(exited, Task.Delay(500)).ConfigureAwait(false);
                        Observe(exchange);
                        return Result(winner == cancelled.Task ? "cancelled" : "timeout");
                    }
                    await exchange.ConfigureAwait(false);
                    if (process.ExitCode != 0) return Result("failed");
                    return new Reply { Status = "ok", Output = await output.ConfigureAwait(false) };
                }
                catch
                {
                    Kill(process);
                    if (exchange != null) Observe(exchange);
                    // Never expose provider stderr, a command buffer, keys, or paths.
                    return Result("failed");
                }
                finally { deadline.Cancel(); }
            }
        }

        private static Reply Result(string status) { return new Reply { Status = status, Output = "" }; }

        private static void Kill(Process process)
        {
            try
            {
                if (!process.HasExited)
                {
                    var killTree = typeof(Process).GetMethod("Kill", new[] { typeof(bool) });
                    if (killTree != null) killTree.Invoke(process, new object[] { true });
                    else process.Kill(); // .NET Framework: parent only, no shell involved.
                }
            }
            catch { try { if (!process.HasExited) process.Kill(); } catch { } }
        }

        private static void Observe(Task task)
        {
            task.ContinueWith(t => { var ignored = t.Exception; }, CancellationToken.None,
                TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously,
                TaskScheduler.Default);
        }

        private static async Task WriteInputAsync(Stream stream, string request)
        {
            // Write bytes directly: Framework's redirected stdin defaults to the
            // console encoding, which would corrupt Chinese and supplementary text.
            var bytes = new UTF8Encoding(false, true).GetBytes(request);
            await stream.WriteAsync(bytes, 0, bytes.Length).ConfigureAwait(false);
            await stream.FlushAsync().ConfigureAwait(false);
            stream.Close();
        }

        private static Task WaitForExitAsync(Process process)
        {
            var exited = new TaskCompletionSource<bool>();
            process.EnableRaisingEvents = true;
            process.Exited += (sender, args) => exited.TrySetResult(true);
            if (process.HasExited) exited.TrySetResult(true);
            return exited.Task;
        }

        private static void ConfigureArguments(ProcessStartInfo start, string[] arguments)
        {
            var argumentList = typeof(ProcessStartInfo).GetProperty("ArgumentList");
            if (argumentList != null)
            {
                var list = (System.Collections.Generic.ICollection<string>)argumentList.GetValue(start, null);
                foreach (var argument in arguments) list.Add(argument);
            }
            else
            {
                // Windows PowerShell/.NET Framework lacks ArgumentList. Quote
                // each argv element using Windows CRT rules, never a shell parser.
                var quoted = new string[arguments.Length];
                for (int i = 0; i < arguments.Length; i++) quoted[i] = QuoteArgument(arguments[i]);
                start.Arguments = String.Join(" ", quoted);
            }
        }

        public static string QuoteArgument(string argument)
        {
            if (argument == null || argument.IndexOf('\0') >= 0)
                throw new ArgumentException("Invalid argument.");
            var result = new StringBuilder("\"");
            int slashes = 0;
            foreach (char c in argument)
            {
                if (c == '\\') { slashes++; continue; }
                result.Append('\\', c == '\"' ? slashes * 2 + 1 : slashes);
                result.Append(c);
                slashes = 0;
            }
            result.Append('\\', slashes * 2);
            return result.Append('\"').ToString();
        }

        private static async Task<string> ReadBoundedAsync(StreamReader reader, bool retain)
        {
            var result = new StringBuilder();
            var buffer = new char[4096];
            int total = 0, count;
            while ((count = await reader.ReadAsync(buffer, 0, buffer.Length).ConfigureAwait(false)) != 0)
            {
                total += count;
                if (total > 65536) throw new InvalidDataException("Adapter response limit exceeded.");
                if (retain) result.Append(buffer, 0, count);
            }
            return result.ToString();
        }
    }

    public static class Text
    {
        public static long NowMilliseconds
        {
            get { return (long)(Stopwatch.GetTimestamp() * (1000.0 / Stopwatch.Frequency)); }
        }

        private static readonly UTF8Encoding Utf8 = new UTF8Encoding(false, true);

        public static int ByteCursor(string buffer, int utf16Cursor)
        {
            if (buffer == null || utf16Cursor < 0 || utf16Cursor > buffer.Length)
                throw new ArgumentException("Invalid cursor.");
            // Strict encoding rejects unpaired surrogates, including a cursor
            // between the halves of an otherwise valid supplementary character.
            Utf8.GetByteCount(buffer);
            return Utf8.GetByteCount(buffer.Substring(0, utf16Cursor));
        }

        public static bool Valid(string text, int maximumBytes)
        {
            if (text == null) return false;
            try { if (Utf8.GetByteCount(text) > maximumBytes) return false; }
            catch (EncoderFallbackException) { return false; }
            foreach (char c in text)
            {
                if (Char.IsControl(c) || c == '\u061c' || (c >= '\u200b' && c <= '\u200f') ||
                    (c >= '\u2028' && c <= '\u202e') || (c >= '\u2060' && c <= '\u206f') || c == '\ufeff')
                    return false;
            }
            return true;
        }
    }
}
