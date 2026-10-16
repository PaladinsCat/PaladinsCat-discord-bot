import io
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from full_history_credential_boundary import credential_in_blob, path_gate


class HistoryChecks(unittest.TestCase):
    def test_credentials_split_across_chunks_are_detected_without_output(self):
        value = b"prefix " + b"github_pat_" + b"a" * 25 + b" suffix"
        self.assertTrue(credential_in_blob(io.BytesIO(value), len(value), 7))
        plain = b"documentation and ordinary Git metadata"
        self.assertFalse(credential_in_blob(io.BytesIO(plain), len(plain), 7))

    def test_truncated_object_never_passes(self):
        with self.assertRaises(ValueError): credential_in_blob(io.BytesIO(b"truncated"), 100)

    def test_historical_templates_and_nested_secret_boundary(self):
        for path in (".env.example", "docs/secrets/README.md"):
            path_gate(path)
        for path in ("local/runtime/account", "a/env/backend.conf", "a/private.key", "a/.env.prod", "a/.env.production.example"):
            with self.assertRaises(ValueError): path_gate(path)


if __name__ == "__main__": unittest.main()
