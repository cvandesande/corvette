"""Check the parity reference Job draft against the live manifest of record."""
import re
import shutil
import subprocess
import sys
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
DEFAULT_JOB = HERE / "window1-reference-job.yaml"
DEFAULT_LIVE = Path.home() / "dockers/kubernetes/tirnanog/frigate-corvette-media.yaml"
FORBIDDEN_WORDS = ("kubectl", "helm", "curl")
DIGEST_RE = re.compile(r"@sha256:[0-9a-f]{64}$")
EXACT_VOLUMES = {"config", "work", "script"}
WRITE_COMMANDS = ("tee", "touch", "mkdir", "rm", "mv", "cp")
REDIRECT_RE = re.compile(r">{1,2}\s*(\"[^\"]*\"|[^\s;&|]+)")
WRITE_RE = re.compile(r"\b(" + "|".join(WRITE_COMMANDS) + r")\b")
SEGMENT_RE = re.compile(r"[;&|]{1,2}|\n")


class Failure(Exception):
    pass


def mapping(value, what):
    if not isinstance(value, dict):
        raise Failure(f"{what} is missing or not a mapping")
    return value


def seq(value, what):
    if not isinstance(value, list):
        raise Failure(f"{what} is missing or not a list")
    return value


def one_line(text):
    return " ".join(str(text).split())


class Ctx:
    def __init__(self, job_text, job_docs, live_sts, live_pod_spec, live_container):
        self.job_text = job_text
        self.job_docs = job_docs
        self.live_sts = live_sts
        self.live_pod_spec = live_pod_spec
        self.live_container = live_container


def load_yaml(path, what):
    try:
        text = path.read_text()
    except OSError as exc:
        raise Failure(f"{what} file is unreadable: {path}") from exc
    try:
        docs = list(yaml.safe_load_all(text))
    except yaml.YAMLError as exc:
        raise Failure(f"{what} file is unparseable YAML: {path}: {one_line(exc)}") from exc
    return text, docs


def live_frigate(docs):
    for doc in docs:
        if not isinstance(doc, dict) or doc.get("kind") != "StatefulSet":
            continue
        meta = doc.get("metadata")
        if not isinstance(meta, dict) or meta.get("name") != "frigate":
            continue
        spec = doc.get("spec")
        template = spec.get("template") if isinstance(spec, dict) else None
        pod_spec = template.get("spec") if isinstance(template, dict) else None
        containers = pod_spec.get("containers") if isinstance(pod_spec, dict) else None
        if isinstance(containers, list):
            for container in containers:
                if isinstance(container, dict) and container.get("name") == "frigate":
                    return doc, pod_spec, container
    raise Failure("live manifest holds no StatefulSet frigate with a container frigate")


def live_config_claim(ctx):
    for volume in seq(ctx.live_pod_spec.get("volumes"), "live volumes"):
        if isinstance(volume, dict) and volume.get("name") == "config":
            claim = mapping(volume.get("persistentVolumeClaim"), "live config volume").get("claimName")
            if not isinstance(claim, str) or not claim:
                raise Failure("live config volume has no claimName")
            return claim
    raise Failure("live manifest has no volume named config")


def job_doc(ctx):
    if not ctx.job_docs:
        raise Failure("draft file holds no YAML document")
    return mapping(ctx.job_docs[0], "draft document")


def container_and_pod(ctx):
    doc = job_doc(ctx)
    template = mapping(mapping(doc.get("spec"), "Job spec").get("template"), "Job template")
    pod_spec = mapping(template.get("spec"), "Job pod spec")
    containers = seq(pod_spec.get("containers"), "pod containers")
    named = [c for c in containers if isinstance(c, dict) and c.get("name") == "reference"]
    if len(named) != 1:
        raise Failure(f"expected exactly one container named reference, found {len(named)}")
    return named[0], pod_spec


def volumes_of(pod_spec):
    volumes = {}
    for volume in seq(pod_spec.get("volumes"), "pod volumes"):
        if not isinstance(volume, dict):
            raise Failure("a volumes entry is not a mapping")
        volumes[volume.get("name")] = volume
    return volumes


def script_text(container):
    command = container.get("command")
    if isinstance(command, list) and len(command) == 3 and isinstance(command[2], str):
        return command[2]
    raise Failure("command holds no shell script string to inspect")


def shellcheck_problems(script):
    exe = shutil.which("shellcheck")
    if exe is None:
        raise Failure("shellcheck is not on PATH")
    try:
        proc = subprocess.run(
            [exe, "-s", "bash", "-"],
            input=script,
            text=True,
            capture_output=True,
            timeout=60,
        )
    except subprocess.TimeoutExpired as exc:
        raise Failure("shellcheck timed out") from exc
    if proc.returncode != 0:
        raise Failure(f"shellcheck exited {proc.returncode}: {one_line(proc.stdout + proc.stderr)}")


def writable_targets(script):
    script = script.replace("\\\n", "")
    tokens = []
    for match in REDIRECT_RE.finditer(script):
        tokens.append(match.group(1).strip('"'))
    for segment in SEGMENT_RE.split(script):
        found = WRITE_RE.search(segment)
        if found is None:
            continue
        for token in segment[found.end():].split():
            if not token.startswith("-"):
                tokens.append(token.strip('"'))
    return tokens


def rule_single_job_document(ctx):
    if len(ctx.job_docs) != 1:
        raise Failure(f"draft holds {len(ctx.job_docs)} YAML documents, expected exactly one")
    doc = job_doc(ctx)
    if doc.get("apiVersion") != "batch/v1" or doc.get("kind") != "Job":
        raise Failure(
            f"draft is apiVersion {doc.get('apiVersion')!r} kind {doc.get('kind')!r}, expected batch/v1 Job"
        )
    _, pod_spec = container_and_pod(ctx)
    containers = seq(pod_spec.get("containers"), "pod containers")
    if len(containers) != 1:
        raise Failure(f"pod template declares {len(containers)} containers, expected exactly one named reference")
    if pod_spec.get("initContainers") is not None:
        raise Failure("pod spec declares initContainers")


def rule_image_matches_live(ctx):
    container, _ = container_and_pod(ctx)
    image = container.get("image")
    if not isinstance(image, str):
        raise Failure("reference container declares no image string")
    live_image = ctx.live_container.get("image")
    if image != live_image:
        raise Failure(f"image {image} is not the live image {live_image}")
    if not DIGEST_RE.search(image):
        raise Failure(f"image {image} does not end in @sha256: plus 64 lowercase hex characters")


def rule_namespace_and_pull_secrets_match_live(ctx):
    meta = mapping(job_doc(ctx).get("metadata"), "draft metadata")
    live_meta = mapping(ctx.live_sts.get("metadata"), "live StatefulSet metadata")
    if meta.get("namespace") != live_meta.get("namespace"):
        raise Failure(f"namespace {meta.get('namespace')!r} is not the live namespace {live_meta.get('namespace')!r}")
    _, pod_spec = container_and_pod(ctx)
    if pod_spec.get("imagePullSecrets") != ctx.live_pod_spec.get("imagePullSecrets"):
        raise Failure(f"imagePullSecrets {pod_spec.get('imagePullSecrets')!r} differ from the live {ctx.live_pod_spec.get('imagePullSecrets')!r}")


def rule_config_volume_read_only(ctx):
    container, pod_spec = container_and_pod(ctx)
    problems = config_mount_problems(container, volumes_of(pod_spec), ctx)
    if problems:
        raise Failure("; ".join(problems))


def config_mount_problems(container, volumes, ctx):
    """Every way the config mount departs from a read-only view of the live config volume."""
    problems = []
    mounts = seq(container.get("volumeMounts"), "volumeMounts")
    for mount in mounts:
        if not isinstance(mount, dict):
            raise Failure("a volumeMounts entry is not a mapping")
        volume = volumes.get(mount.get("name"))
        if isinstance(volume, dict) and "persistentVolumeClaim" in volume and mount.get("readOnly") is not True:
            problems.append(f"volume {mount.get('name')!r} is a PVC but its mount lacks readOnly: true")
    config = volumes.get("config")
    if not isinstance(config, dict) or "persistentVolumeClaim" not in config:
        problems.append("no volume named config backed by a persistentVolumeClaim")
    else:
        claim = config["persistentVolumeClaim"].get("claimName")
        if claim != live_config_claim(ctx):
            problems.append(f"config claimName {claim!r} is not the live claim name {live_config_claim(ctx)!r}")
    config_mounts = [m for m in mounts if isinstance(m, dict) and m.get("name") == "config"]
    if len(config_mounts) != 1 or config_mounts[0].get("mountPath") != "/config":
        problems.append("config is not mounted at /config exactly once")
    return problems


def rule_volumes_are_exactly_config_work_script(ctx):
    _, pod_spec = container_and_pod(ctx)
    volumes = volumes_of(pod_spec)
    if set(volumes) != EXACT_VOLUMES:
        raise Failure(f"volume names {sorted(map(str, volumes))} are not exactly config, script, work")
    for name, volume in volumes.items():
        if "hostPath" in volume:
            raise Failure(f"volume {name!r} uses hostPath")
    if "emptyDir" not in volumes["work"]:
        raise Failure("work is not an emptyDir")
    config_map = volumes["script"].get("configMap")
    if not isinstance(config_map, dict) or config_map.get("name") != "parity-reference-script":
        raise Failure("script is not a configMap named parity-reference-script")


def rule_gpu_claim_matches_live(ctx):
    container, _ = container_and_pod(ctx)
    resources = mapping(container.get("resources"), "reference resources")
    blocks = (("limits", mapping(resources.get("limits"), "resources.limits")),
              ("requests", mapping(resources.get("requests"), "resources.requests")))
    for label, block in blocks:
        if list(block) != ["amd.com/gpu"]:
            raise Failure(f"{label} is {dict(block)!r}, expected exactly one amd.com/gpu entry")
        if block["amd.com/gpu"] not in (1, "1"):
            raise Failure(f"{label} amd.com/gpu is {block['amd.com/gpu']!r}, not 1")
    live_limits = mapping(mapping(ctx.live_container.get("resources"), "live resources").get("limits"), "live limits")
    if live_limits.get("amd.com/gpu") not in (1, "1"):
        raise Failure(f"live amd.com/gpu limit is {live_limits.get('amd.com/gpu')!r}, not 1")


def rule_no_host_access_or_privilege(ctx):
    container, pod_spec = container_and_pod(ctx)
    for key in ("hostNetwork", "hostPID", "hostIPC"):
        if pod_spec.get(key) is True:
            raise Failure(f"pod spec sets {key} true")
    sc = mapping(container.get("securityContext"), "reference securityContext")
    if sc.get("privileged") is not False:
        raise Failure(f"privileged is {sc.get('privileged')!r}, not false")
    if sc.get("allowPrivilegeEscalation") is not False:
        raise Failure(f"allowPrivilegeEscalation is {sc.get('allowPrivilegeEscalation')!r}, not false")
    caps = mapping(sc.get("capabilities"), "reference capabilities")
    if caps.get("add"):
        raise Failure(f"capabilities.add is {caps['add']!r}, expected none")
    if "ALL" not in seq(caps.get("drop"), "capabilities.drop"):
        raise Failure("capabilities.drop does not contain ALL")


def rule_command_is_checked_shell(ctx):
    container, _ = container_and_pod(ctx)
    command = seq(container.get("command"), "command")
    if len(command) != 3 or not all(isinstance(part, str) for part in command):
        raise Failure(f"command is {command!r}, expected three strings")
    if command[0] != "/bin/bash" or command[1] != "-c":
        raise Failure(f"command starts {command[0]!r} {command[1]!r}, expected /bin/bash -c")
    shellcheck_problems(command[2])
    for word in FORBIDDEN_WORDS:
        if word in ctx.job_text:
            raise Failure(f"draft file contains the word {word!r}")


def rule_script_does_not_write_config(ctx):
    container, _ = container_and_pod(ctx)
    script = script_text(container)
    for match in re.finditer(r"/config", script):
        if not script.startswith("/model_cache/", match.end()):
            raise Failure("/config appears outside /config/model_cache/ paths")
    for token in writable_targets(script):
        if token.startswith(("/config", "$model", "${model")):
            raise Failure(f"a redirection or file command takes the path {token}")


RULES = (
    ("single-job-document", rule_single_job_document),
    ("image-matches-live", rule_image_matches_live),
    ("namespace-and-pull-secrets-match-live", rule_namespace_and_pull_secrets_match_live),
    ("config-volume-read-only", rule_config_volume_read_only),
    ("volumes-are-exactly-config-work-script", rule_volumes_are_exactly_config_work_script),
    ("gpu-claim-matches-live", rule_gpu_claim_matches_live),
    ("no-host-access-or-privilege", rule_no_host_access_or_privilege),
    ("command-is-checked-shell", rule_command_is_checked_shell),
    ("script-does-not-write-config", rule_script_does_not_write_config),
)


def main(argv):
    job_path = Path(argv[1]) if len(argv) > 1 else DEFAULT_JOB
    live_path = Path(argv[2]) if len(argv) > 2 else DEFAULT_LIVE
    try:
        job_text, job_docs = load_yaml(job_path, "Job draft")
        _, live_docs = load_yaml(live_path, "Live manifest")
        live_sts, live_pod_spec, live_container = live_frigate(live_docs)
    except Failure as exc:
        print(f"error: {one_line(exc)}", file=sys.stderr)
        return 2
    ctx = Ctx(job_text, job_docs, live_sts, live_pod_spec, live_container)
    failed = False
    for name, rule in RULES:
        try:
            rule(ctx)
        except Failure as exc:
            failed = True
            print(f"FAIL {name}: {one_line(exc)}")
        else:
            print(f"ok {name}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
