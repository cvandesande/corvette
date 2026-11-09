import shutil
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CHECK = HERE / "check_reference_job.py"
DRAFT = HERE / "window1-reference-job.yaml"
LIVE = Path.home() / "dockers/kubernetes/tirnanog/frigate-corvette-media.yaml"
SCRATCH_ROOT = ROOT / "target" / "tmp" / "parity-job"


def run_check(job, live):
    return subprocess.run(
        [sys.executable, "-I", str(CHECK), str(job), str(live)],
        capture_output=True,
        text=True,
        timeout=60,
    )


def ok_count(stdout):
    return sum(1 for line in stdout.splitlines() if line.startswith("ok "))


class ShippedDraft(unittest.TestCase):
    def test_the_shipped_draft_passes_against_the_live_manifest(self):
        proc = run_check(DRAFT, LIVE)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(ok_count(proc.stdout), 9, proc.stdout)
        self.assertNotIn("FAIL", proc.stdout)


class MutatedDraft(unittest.TestCase):
    def setUp(self):
        self.scratch = SCRATCH_ROOT / self.id().rsplit(".", 1)[-1]
        if self.scratch.exists():
            shutil.rmtree(self.scratch)
        self.scratch.mkdir(parents=True)
        self.draft = self.scratch / "draft.yaml"
        self.live = self.scratch / "live.yaml"
        self.draft_text = DRAFT.read_text()
        self.live_text = LIVE.read_text()
        self.draft.write_text(self.draft_text)
        self.live.write_text(self.live_text)

    def edit_draft(self, old, new):
        edited = self.draft_text.replace(old, new, 1)
        self.assertNotEqual(edited, self.draft_text)
        self.draft.write_text(edited)

    def edit_live(self, old, new):
        edited = self.live_text.replace(old, new, 1)
        self.assertNotEqual(edited, self.live_text)
        self.live.write_text(edited)

    def check(self):
        return run_check(self.draft, self.live)

    def assert_fails_rule(self, proc, rule):
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertIn(f"FAIL {rule}", proc.stdout)

    def test_a_writable_config_mount_fails(self):
        self.edit_draft("mountPath: /config\n              readOnly: true", "mountPath: /config")
        proc = self.check()
        self.assert_fails_rule(proc, "config-volume-read-only")
        self.assertEqual(ok_count(proc.stdout), 8, proc.stdout)

    def test_a_second_volume_fails(self):
        self.edit_draft(
            "        - name: work\n          emptyDir: {}",
            "        - name: extra\n          emptyDir: {}\n        - name: work\n          emptyDir: {}",
        )
        self.assert_fails_rule(self.check(), "volumes-are-exactly-config-work-script")

    def test_a_bare_tag_image_fails(self):
        self.edit_draft("@sha256:a8164fa8384b05bfd130d006f77aacb5918a646aa2a1b14fe1626281781a798a", "")
        self.assert_fails_rule(self.check(), "image-matches-live")

    def test_a_changed_live_image_fails(self):
        self.edit_live("781a798a", "781a798b")
        self.assert_fails_rule(self.check(), "image-matches-live")

    def test_a_gpu_limit_of_two_fails(self):
        self.edit_draft("limits:\n              amd.com/gpu: 1", "limits:\n              amd.com/gpu: 2")
        self.assert_fails_rule(self.check(), "gpu-claim-matches-live")

    def test_a_missing_gpu_request_fails(self):
        self.edit_draft("            requests:\n              amd.com/gpu: 1\n", "")
        self.assert_fails_rule(self.check(), "gpu-claim-matches-live")

    def test_a_privileged_container_fails(self):
        self.edit_draft("privileged: false", "privileged: true")
        self.assert_fails_rule(self.check(), "no-host-access-or-privilege")

    def test_a_broken_shell_command_fails(self):
        self.edit_draft('sleep "$hold_seconds"', "sleep $HOME")
        proc = self.check()
        self.assert_fails_rule(proc, "command-is-checked-shell")
        self.assertIn("SC2086", proc.stdout)

    def test_a_cluster_command_in_the_file_fails(self):
        self.edit_draft("defaultMode: 0444\n", "defaultMode: 0444\n# kubectl cp\n")
        self.assert_fails_rule(self.check(), "command-is-checked-shell")

    def test_a_write_to_config_fails(self):
        self.edit_draft("hold_seconds=1800\n", "hold_seconds=1800\n              touch /config/x\n")
        self.assert_fails_rule(self.check(), "script-does-not-write-config")

    def test_a_wrong_claim_name_fails(self):
        self.edit_draft("claimName: frigate-config", "claimName: frigate-media")
        self.assert_fails_rule(self.check(), "config-volume-read-only")

    def test_two_documents_fail(self):
        self.edit_draft("defaultMode: 0444\n", "defaultMode: 0444\n---\nkind: ConfigMap\n")
        self.assert_fails_rule(self.check(), "single-job-document")

    def test_a_second_container_fails(self):
        self.edit_draft(
            "      containers:\n",
            "      containers:\n        - name: sidecar\n"
            "          image: cvandesande/frigate-vulkan:py313-20261106@sha256:a8164fa8384b05bfd130d006f77aacb5918a646aa2a1b14fe1626281781a798a\n"
            "          volumeMounts:\n            - name: config\n              mountPath: /config\n",
        )
        self.assert_fails_rule(self.check(), "single-job-document")

    def test_a_continued_rm_of_the_model_fails(self):
        self.edit_draft(
            "              hold_seconds=1800\n",
            "              hold_seconds=1800\n              rm \\\n                \"$model\"\n",
        )
        self.assert_fails_rule(self.check(), "script-does-not-write-config")

    def test_a_continued_redirect_to_the_model_fails(self):
        self.edit_draft(
            "              hold_seconds=1800\n",
            "              hold_seconds=1800\n              echo x > \\\n                \"$model\"\n",
        )
        self.assert_fails_rule(self.check(), "script-does-not-write-config")

    def write_bad_draft(self):
        bad = self.scratch / "bad.yaml"
        bad.write_text("a: [\n")
        return bad

    def test_unusable_files_exit_2(self):
        cases = [(self.scratch / "absent.yaml", self.live), (self.write_bad_draft(), self.live)]
        self.edit_live(
            "kind: StatefulSet\nmetadata:\n  name: frigate",
            "kind: StatefulSet\nmetadata:\n  name: frigate-renamed",
        )
        cases.append((self.draft, self.live))
        for job, live in cases:
            proc = run_check(job, live)
            self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
            self.assertEqual(proc.stdout, "")
            lines = [line for line in proc.stderr.splitlines() if line.strip()]
            self.assertEqual(len(lines), 1, proc.stderr)
            self.assertTrue(lines[0].startswith("error:"), proc.stderr)


if __name__ == "__main__":
    unittest.main()
