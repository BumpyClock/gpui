"""Executable hook fixtures. Stub toolchain outcomes are not native GPUI acceptance."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2]
ZERO = '0' * 40


class HookTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='gpui hook fixture ')
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.repo = base / 'repo with spaces'
        self.repo.mkdir()
        binary = base / 'bin'
        binary.mkdir()
        self.log = base / 'calls'
        self.env = {**os.environ, 'PATH': f'{binary}{os.pathsep}{os.environ["PATH"]}',
                    'CALL_LOG': str(self.log), 'GIT_CONFIG_NOSYSTEM': '1',
                    'GIT_CONFIG_GLOBAL': str(base / 'no-global-config'),
                    'GIT_AUTHOR_NAME': 'Fixture', 'GIT_AUTHOR_EMAIL': 'fixture@example.invalid',
                    'GIT_COMMITTER_NAME': 'Fixture', 'GIT_COMMITTER_EMAIL': 'fixture@example.invalid'}
        stub = binary / 'rustup'
        stub.write_text('#!/bin/sh\nset -eu\necho "$*" >> "$CALL_LOG"\n'
                        'case "$*" in *"cargo fmt"*) exit "${FMT_EXIT:-0}" ;; '
                        '*"cargo check"*) exit "${CHECK_EXIT:-0}" ;; '
                        '*"rustc --version"*) exit "${RUSTC_EXIT:-0}" ;; esac\n')
        stub.chmod(0o755)
        for path in ['.githooks/pre-push', 'scripts/setup-hooks.sh']:
            dest = self.repo / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(SOURCE / path, dest)
        (self.repo / 'source with spaces.rs').write_text('fn main() {}\n')
        self.git('init', '-q')
        self.git('add', '.')
        self.git('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'fixture')
        self.head = self.git('rev-parse', 'HEAD').stdout.strip()

    def run_command(self, args, **kwargs):
        return subprocess.run(args, cwd=self.repo, env=self.env, text=True, capture_output=True, **kwargs)

    def git(self, *args):
        result = self.run_command(['git', *args])
        self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def hook(self, data=None):
        if data is None:
            data = f'refs/heads/main {self.head} refs/heads/main {ZERO}\n'
        return self.run_command([str(self.repo / '.githooks/pre-push')], input=data)

    def test_full_checks_for_new_unknown_empty_and_multiple_refs(self):
        for data in ['', None, f'refs/heads/main {self.head} refs/heads/main {"f" * 40}\n',
                     f'refs/heads/main {self.head} refs/heads/main {ZERO}\nrefs/heads/new {self.head} refs/heads/new {ZERO}\n']:
            result = self.hook(data)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('run 1.95.0 cargo fmt --all -- --check', self.log.read_text())
            self.assertIn('run 1.95.0 cargo check --locked --offline --workspace --all-targets', self.log.read_text())

    def test_deletions_and_mixed_updates(self):
        deletion = f'(delete) {ZERO} refs/heads/old {self.head}\n'
        self.assertEqual(self.hook(deletion).returncode, 0)
        self.assertFalse(self.log.exists())
        self.assertEqual(self.hook(deletion + f'refs/heads/new {self.head} refs/heads/new {ZERO}\n').returncode, 0)
        self.assertTrue(self.log.exists())

    def test_malformed_dirty_different_commit_and_rename(self):
        self.assertNotEqual(self.hook('invalid\n').returncode, 0)
        self.assertNotEqual(self.hook(f'main invalid main {ZERO}\n').returncode, 0)
        (self.repo / 'untracked.rs').write_text('')
        self.assertNotEqual(self.hook().returncode, 0)
        self.git('add', '.')
        self.git('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'next')
        self.assertNotEqual(self.hook().returncode, 0)
        previous = self.git('rev-parse', 'HEAD').stdout.strip()
        self.git('mv', 'source with spaces.rs', 'renamed source.rs')
        self.git('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'rename')
        self.head = self.git('rev-parse', 'HEAD').stdout.strip()
        self.assertEqual(self.hook(f'main {self.head} main {previous}\n').returncode, 0)

    def test_missing_toolchain_fails_without_bootstrap(self):
        self.env['RUSTC_EXIT'] = '1'
        result = self.hook()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Bootstrap it explicitly', result.stderr)
        self.assertEqual(self.log.read_text(), 'run 1.95.0 rustc --version\n')

    def test_nonzero_propagation(self):
        self.env['FMT_EXIT'] = '17'
        self.assertEqual(self.hook().returncode, 17)
        self.assertNotIn('cargo check', self.log.read_text())
        self.env['FMT_EXIT'] = '0'
        self.env['CHECK_EXIT'] = '19'
        self.assertEqual(self.hook().returncode, 19)

    def test_installer_conflicts_idempotence_and_linked_fixture(self):
        installer = [str(self.repo / 'scripts/setup-hooks.sh')]
        self.git('config', 'core.hooksPath', 'unrelated hooks')
        self.assertNotEqual(self.run_command(installer).returncode, 0)
        self.assertEqual(self.git('config', 'core.hooksPath').stdout.strip(), 'unrelated hooks')
        self.git('config', 'core.hooksPath', '')
        self.assertNotEqual(self.run_command(installer).returncode, 0)
        self.git('config', '--unset', 'core.hooksPath')
        active = self.repo / '.git/hooks/pre-commit'
        active.write_text('#!/bin/sh\nexit 0\n')
        active.chmod(0o755)
        self.assertNotEqual(self.run_command(installer).returncode, 0)
        active.unlink()
        linked = Path(self.temp.name) / 'linked fixture'
        self.git('worktree', 'add', '-qb', 'linked', str(linked))
        result = subprocess.run([str(linked / 'scripts/setup-hooks.sh')], cwd=linked, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.run_command(installer).returncode, 0)
        self.assertEqual(self.git('config', '--local', 'core.hooksPath').stdout.strip(), '.githooks')
        (self.repo / '.githooks/pre-push').chmod(0o644)
        self.assertNotEqual(self.run_command(installer).returncode, 0)


if __name__ == '__main__':
    unittest.main()
