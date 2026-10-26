"""Offline protocol parity checks for the Bash and shared native transports."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
bash = load('bash_bridge', 'integrations/bash/bridge.py')
shared = load('shared_bridge', 'integrations/shared/bridge.py')

class V2Contract(unittest.TestCase):
    def fixtures(self):
        req = dict(protocol=2, request_id='fixture', shell=dict(kind='cmd', version='synthetic'), cwd='/do-not-read', buffer='启动监控 suffix', cursor=6)
        replacement = 'echo 世界😀'
        reply = dict(protocol=2, request_id=req['request_id'], buffer=req['buffer'], cursor=req['cursor'], insertion='', replacement=replacement, result=replacement, result_cursor=len(replacement.encode()), risk_review=dict(warnings=[], incomplete=False), additional_acceptance_required=False)
        return req, reply

    def validate(self, req, reply):
        b = bash.validate(json.dumps(reply).encode(), req)
        s, risk = shared.check_response(req, reply)
        self.assertEqual((b[0], b[1], b[2]), (s['result'], s['result_cursor'], risk))
        return risk

    def test_replacement_unicode_and_frame(self):
        req, reply = self.fixtures()
        self.validate(req, reply)
        self.assertEqual(shared.check_request(req), req)
        frame = shared.response_frame(req, reply).splitlines()
        self.assertEqual(frame[0], 'RLA2R')
        self.assertEqual(bytes.fromhex(frame[4]).decode(), reply['replacement'])

    def test_v1_rejects_replacement_and_v2_rejects_malformed(self):
        changes = [('protocol', 1), ('replacement', None), ('replacement', 'x' * 8193), ('replacement', '```echo```'), ('replacement', 'echo\nBAD'), ('insertion', 'x'), ('result', 'unrelated'), ('result_cursor', True), ('result_cursor', 7), ('buffer', 'edited'), ('cursor', 3)]
        for key, value in changes:
            req, reply = self.fixtures()
            reply[key] = value
            if key == 'protocol': req[key] = value
            with self.subTest(key=key), self.assertRaises((bash.Failure, ValueError)):
                bash.validate(json.dumps(reply).encode(), req)
            with self.subTest(key=key), self.assertRaises(ValueError):
                shared.check_response(req, reply)

    def test_risk_never_bypassed(self):
        for kind in ['warnings', 'incomplete', 'additional_acceptance_required']:
            req, reply = self.fixtures()
            if kind == 'warnings': reply['risk_review'][kind] = ['review me']
            elif kind == 'incomplete': reply['risk_review'][kind] = True
            else: reply[kind] = True
            self.assertTrue(self.validate(req, reply))

    def test_v1_and_v2_insertions_keep_suffix(self):
        for protocol in [1, 2]:
            req, reply = self.fixtures()
            req['protocol'] = reply['protocol'] = protocol
            del reply['replacement']
            reply['insertion'] = '界'
            source = req['buffer'].encode(); point = req['cursor']
            reply['result'] = (source[:point] + '界'.encode() + source[point:]).decode()
            reply['result_cursor'] = point + 3
            self.validate(req, reply)
            reply['result'] = 'rewritten'
            with self.assertRaises(bash.Failure): bash.validate(json.dumps(reply).encode(), req)
            with self.assertRaises(ValueError): shared.check_response(req, reply)

if __name__ == '__main__': unittest.main()
