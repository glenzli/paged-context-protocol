import importlib.util
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("install_service", Path(__file__).with_name("install-service.py"))
service = importlib.util.module_from_spec(spec)
spec.loader.exec_module(service)


class ServiceInstallTests(unittest.TestCase):
    def test_first_completed_poll_can_take_more_than_thirty_seconds(self):
        with patch.object(service.time, 'monotonic', side_effect=[0, 1, 31, 61]), \
                patch.object(service.time, 'sleep'), \
                patch.object(service, 'run_quiet', side_effect=[Mock(returncode=2),
                             Mock(returncode=2), Mock(returncode=0)]):
            self.assertTrue(service.wait_for_ready('/tunnel-client', 4319))

    def test_readiness_wait_is_bounded(self):
        with patch.object(service.time, 'monotonic', side_effect=[0, 1, 91]), \
                patch.object(service.time, 'sleep'), \
                patch.object(service, 'run_quiet', return_value=Mock(returncode=2)):
            self.assertFalse(service.wait_for_ready('/tunnel-client', 4319))

    PROFILE = '''config_version: 1
control_plane:
  base_url: "https://api.openai.com"
  tunnel_id: "tunnel_test"
  api_key: "env:CONTROL_PLANE_API_KEY"
health:
  # Keep this comment.
  listen_addr: "127.0.0.1:8080"
admin_ui:
  open_browser: true
mcp:
  commands:
    - channel: main
      command: '"/Library/Application Support/PCP/pcp-chatgpt-mcp"'
'''

    def test_service_profile_does_not_require_foreground_environment(self):
        key = Path("/Library/Application Support/PCP/runtime-api-key")
        result = service.service_profile(self.PROFILE, key, 4319)
        self.assertNotIn("env:CONTROL_PLANE_API_KEY", result)
        self.assertIn(f'  api_key: "file:{key}"', result)
        self.assertIn('  listen_addr: "127.0.0.1:4319"', result)
        self.assertIn('  open_browser: false', result)
        self.assertEqual(result.split('mcp:')[1], self.PROFILE.split('mcp:')[1])
        self.assertIn('# Keep this comment.', result)
        self.assertIn('tunnel_id: "tunnel_test"', result)

    def test_ambiguous_or_unsupported_yaml_fails_closed(self):
        for source in (
            self.PROFILE + 'control_plane:\n  api_key: "env:SECOND_KEY"\n',
            self.PROFILE.replace('  api_key:', '  api_key: "env:SECOND_KEY"\n  api_key:'),
            self.PROFILE.replace('"env:CONTROL_PLANE_API_KEY"', '|\n    env:CONTROL_PLANE_API_KEY'),
            self.PROFILE.replace('control_plane:\n', 'control_plane: &shared\n'),
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                service.service_profile(source, Path('/private/key'), 4319)

    def test_plist_uses_file_reference_and_loopback_without_a_shell(self):
        root = Path("/Users/example/Library/Application Support/PCP")
        config = service.service_plist(root / "tunnel-client", root / "profile.yaml",
                                       root / "runtime-api-key", root / "logs", 4319)
        self.assertEqual(plistlib.loads(plistlib.dumps(config)), config)
        argv = config["ProgramArguments"]
        self.assertEqual(argv[0], str(root / "tunnel-client"))
        self.assertIn(f"file:{root}/runtime-api-key", argv)
        self.assertIn("127.0.0.1:4319", argv)
        self.assertNotIn("CONTROL_PLANE_API_KEY", config["EnvironmentVariables"])
        self.assertTrue(config["RunAtLoad"])
        self.assertTrue(config["KeepAlive"])
        self.assertEqual(config["Umask"], 0o077)

    def test_private_write_replaces_contents_and_permissions(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "key"
            service.private_write(path, b"test-only-placeholder")
            service.check_private_key(path)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                service.check_private_key(path)
            service.private_write(path, b"replacement-placeholder")
            service.check_private_key(path)
            self.assertEqual(path.read_bytes(), b"replacement-placeholder")
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_symlink_and_empty_key_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service.private_write(root / "target", b"unchanged")
            (root / "link").symlink_to(root / "target")
            with self.assertRaises(ValueError):
                service.private_write(root / "link", b"other")
            with self.assertRaises(ValueError):
                service.check_private_key(root / "link")
            service.private_write(root / "empty", b"")
            with self.assertRaises(ValueError):
                service.check_private_key(root / "empty")
            self.assertEqual((root / "target").read_bytes(), b"unchanged")


if __name__ == "__main__":
    unittest.main()
