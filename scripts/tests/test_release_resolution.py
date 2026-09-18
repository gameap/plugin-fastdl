#!/usr/bin/env python3
"""Exercise Linux release selection without network access or service changes."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "install-linux.sh"
FUNCTIONS = SCRIPT.read_text().split("# Pre-flight", 1)[0]
REPOSITORY = "https://github.com/gameap/gameap-fastdl"
DIGEST = "a1" * 32
MOCKS = r'''
uname() {
    case "$1" in
        -s) printf '%s' "$TEST_OS" ;;
        -m) printf '%s' "$TEST_ARCH" ;;
        *) return 1 ;;
    esac
}
curl() {
    printf '%s\n' "${!#}" >> "$TEST_REQUESTS"
    case "${!#}" in
        */releases/latest)
            [ "$TEST_RELEASE_STATUS" = 0 ] || return "$TEST_RELEASE_STATUS"
            printf '%s' "$TEST_RELEASE_URL"
            ;;
        *)
            [ "$TEST_CHECKSUM_STATUS" = 0 ] || return "$TEST_CHECKSUM_STATUS"
            printf '%s' "$TEST_CHECKSUM"
            ;;
    esac
}
'''


class ReleaseResolutionTests(unittest.TestCase):
    def run_script(self, body="resolve_release", **overrides):
        with tempfile.TemporaryDirectory(prefix="fastdl-release-test-") as directory:
            log = Path(directory) / "requests"
            env = dict(os.environ, TEST_OS="Linux", TEST_ARCH="x86_64",
                       TEST_RELEASE_STATUS="0", TEST_CHECKSUM_STATUS="0",
                       TEST_RELEASE_URL=f"{REPOSITORY}/releases/tag/v1.2.3",
                       TEST_CHECKSUM=DIGEST, TEST_REQUESTS=str(log))
            env.update(overrides)
            result = subprocess.run(
                ["/bin/bash", "-c", FUNCTIONS + MOCKS + body],
                env=env, text=True, capture_output=True, check=False,
            )
            requests = log.read_text().splitlines() if log.exists() else []
            return result, requests

    def test_native_architectures_use_one_pinned_release(self):
        for tag in ["v0.0.1", "v1.2.3"]:
            for native, expected in [("x86_64", "amd64"), ("amd64", "amd64"),
                                     ("aarch64", "arm64"), ("arm64", "arm64")]:
                with self.subTest(tag=tag, native=native):
                    asset = f"gameap-fastdl-{tag}-linux-{expected}"
                    result, requests = self.run_script(
                        'resolve_release; printf "RESULT=%s,%s\\n" "$DOWNLOAD_URL" "$SHA256"',
                        TEST_ARCH=native,
                        TEST_RELEASE_URL=f"{REPOSITORY}/releases/tag/{tag}",
                        TEST_CHECKSUM=f"{DIGEST}  {asset}\n",
                    )
                    url = f"{REPOSITORY}/releases/download/{tag}/{asset}"
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn(f"RESULT={url},{DIGEST}", result.stdout)
                    self.assertEqual(requests, [f"{REPOSITORY}/releases/latest", f"{url}.sha256"])

    def test_checksum_formats(self):
        asset = "gameap-fastdl-v1.2.3-linux-amd64"
        for checksum in [DIGEST.upper(), f"{DIGEST}  {asset}\n",
                         f"{DIGEST}\t*{asset}\r\n", f"\n{DIGEST}\n\n"]:
            with self.subTest(checksum=checksum):
                result, _ = self.run_script(
                    'resolve_release; printf "DIGEST=%s\\n" "$SHA256"', TEST_CHECKSUM=checksum,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"DIGEST={DIGEST}", result.stdout)

    def test_missing_or_invalid_checksum_stops_resolution(self):
        for checksum in ["", "z" * 64, "a" * 63, "a" * 65,
                         f"{DIGEST}  another-binary", f"{DIGEST}  ../gameap-fastdl-v1.2.3-linux-amd64",
                         f"{DIGEST}  gameap-fastdl-v0.0.1-linux-amd64",
                         f"{DIGEST}  gameap-fastdl-linux-amd64",
                         f"{DIGEST} extra extra", f"{DIGEST}\n{DIGEST}", "x" * 4097]:
            with self.subTest(checksum=checksum[:80]):
                result, _ = self.run_script(TEST_CHECKSUM=checksum)
                self.assertNotEqual(result.returncode, 0)
        result, _ = self.run_script(TEST_CHECKSUM_STATUS="22")
        self.assertNotEqual(result.returncode, 0)

    def test_missing_release(self):
        for overrides in [dict(TEST_RELEASE_STATUS="22"),
                          dict(TEST_RELEASE_URL=f"{REPOSITORY}/releases")]:
            result, requests = self.run_script(**overrides)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(requests, [f"{REPOSITORY}/releases/latest"])

    def test_unsafe_release_redirects_are_rejected(self):
        for release_url in [
            "http://github.com/gameap/gameap-fastdl/releases/tag/v1.2.3",
            "https://other.example/releases/tag/v1.2.3",
            "https://user:password@github.com/gameap/gameap-fastdl/releases/tag/v1.2.3",
            f"{REPOSITORY}/releases/tag/../v1.2.3",
            f"{REPOSITORY}/releases/tag/v1.2.3?x=y",
            f"{REPOSITORY}/releases/tag/v1.2.3#fragment",
            f"{REPOSITORY}/releases/tag/v1.2.3%0a",
            f"{REPOSITORY}/releases/tag/v1.2.3\nmalicious",
            f"{REPOSITORY}/releases/tag/" + "v" * 129,
        ]:
            with self.subTest(url=release_url):
                result, requests = self.run_script(TEST_RELEASE_URL=release_url)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(requests), 1)

    def test_unsupported_platforms_do_not_download(self):
        for overrides in [dict(TEST_OS="Darwin"), dict(TEST_ARCH="i686"),
                          dict(TEST_ARCH="armv7l")]:
            result, requests = self.run_script(**overrides)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(requests, [])

    def test_explicit_pair_is_preserved_without_network(self):
        result, requests = self.run_script(
            'DOWNLOAD_URL=https://releases.example/custom; SHA256=$TEST_CHECKSUM; '
            'validate_download_options; printf "RESULT=%s,%s\\n" "$DOWNLOAD_URL" "$SHA256"',
            TEST_CHECKSUM=DIGEST.upper(),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"RESULT=https://releases.example/custom,{DIGEST}", result.stdout)
        self.assertEqual(requests, [])

    def test_empty_pair_allows_automatic_resolution(self):
        result, requests = self.run_script("validate_download_options")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(requests, [])

    def test_partial_or_unsafe_overrides_are_rejected(self):
        for body in [
            "DOWNLOAD_URL=https://releases.example/custom; validate_download_options",
            "SHA256=$TEST_CHECKSUM; validate_download_options",
            "DOWNLOAD_URL=http://releases.example/custom; SHA256=$TEST_CHECKSUM; validate_download_options",
            "DOWNLOAD_URL=https://user:pass@releases.example/custom; SHA256=$TEST_CHECKSUM; validate_download_options",
            "DOWNLOAD_URL=https://releases.example/custom; SHA256=bad; validate_download_options",
        ]:
            with self.subTest(body=body):
                result, requests = self.run_script(body)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(requests, [])


if __name__ == "__main__":
    unittest.main()
