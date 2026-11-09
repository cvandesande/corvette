"""Tests for python_reference.py, run with the goldens shell."""

import importlib.util
import shutil
import subprocess
import sys
import types
import unittest
from pathlib import Path
from unittest import mock

import numpy as np

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "parity" / "python_reference.py"
TOOL = ROOT / "target" / "debug" / "corvette-detect-tools"
PARAM = (
    "7767517\n"
    "4 4\n"
    "Input in0 0 1 in0 0=640 1=640 2=3\n"
    "Flatten flat 1 1 in0 flat0\n"
    "Crop crop 1 1 flat0 crop0 0=0 3=420000\n"
    "Reshape out 1 1 crop0 out0 0=8400 1=50 2=1\n"
)


def load_reference():
    spec = importlib.util.spec_from_file_location("python_reference", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


REFERENCE = load_reference()


def scratch(tag):
    path = ROOT / "target" / "tmp" / "parity-reference" / tag
    shutil.rmtree(path, ignore_errors=True)
    path.mkdir(parents=True)
    return path


def tensor_bytes(k):
    pixel = np.arange(640 * 640, dtype=np.int64)
    image = np.empty((640, 640, 3), dtype=np.uint8)
    for channel in range(3):
        image[:, :, channel] = ((pixel * 5 + channel * 85 + k * 13) % 256).reshape(640, 640)
    return image.tobytes()


def write_tensors(in_dir, count):
    for k in range(count):
        (in_dir / f"tensor-{k:05}.rgb").write_bytes(tensor_bytes(k))


def expected_values(raw):
    index = np.arange(420000, dtype=np.int64)
    picked = raw[(index % 409600) * 3 + index // 409600]
    return picked.astype(np.float32) / np.float32(255)


def run_script(args):
    argv = [sys.executable, "-I", str(SCRIPT)] + [str(arg) for arg in args]
    return subprocess.run(argv, capture_output=True, timeout=60)


def run_tool(args):
    if not TOOL.is_file():
        raise AssertionError(f"{TOOL} is missing; build it with cargo first")
    return subprocess.run([str(TOOL)] + [str(arg) for arg in args], capture_output=True, timeout=60)


def held_names(out_dir):
    return sorted(entry.name for entry in out_dir.iterdir())


def _rec_add(rec, entry):
    rec.entries.append(entry)


class _FakeOpt:
    def __init__(self, rec):
        object.__setattr__(self, "rec", rec)

    def __setattr__(self, name, value):
        _rec_add(self.rec, f"{name} = {value}")
        object.__setattr__(self, name, value)


class _FakeInfo:
    def device_name(self):
        return "fake-vulkan-device"


class _FakeExtractor:
    def __init__(self, rec, options):
        self.rec = rec
        self.options = options

    def input(self, name, mat):
        _rec_add(self.rec, f"input({name})")
        return self.options["input_rc"]

    def extract(self, name):
        _rec_add(self.rec, f"extract({name})")
        if self.options["extract_rc"] != 0:
            return (self.options["extract_rc"], None)
        return (0, self.options["extract_value"])


class _FakeNet:
    def __init__(self, rec, options):
        _rec_add(rec, "Net()")
        self.rec = rec
        self.options = options
        self.opt = _FakeOpt(rec)

    def set_vulkan_device(self, device):
        _rec_add(self.rec, f"set_vulkan_device({device})")

    def load_param(self, path):
        _rec_add(self.rec, f"load_param({path})")
        return self.options["param_rc"]

    def load_model(self, path):
        _rec_add(self.rec, f"load_model({path})")
        return self.options["model_rc"]

    def create_extractor(self):
        _rec_add(self.rec, "create_extractor()")
        return _FakeExtractor(self.rec, self.options)


class _FakeMat:
    def __init__(self, rec, array):
        _rec_add(rec, "Mat()")
        rec.mats.append(array)


def fake_ncnn(rec, param_rc=0, model_rc=0, input_rc=0, extract_rc=0, extract_value=None, gpu_count=1, default_gpu=0):
    options = {
        "param_rc": param_rc,
        "model_rc": model_rc,
        "input_rc": input_rc,
        "extract_rc": extract_rc,
        "extract_value": np.zeros(16, dtype=np.float32) if extract_value is None else extract_value,
    }

    def net_factory():
        return _FakeNet(rec, options)

    def mat_factory(array):
        return _FakeMat(rec, array)

    def get_gpu_count():
        _rec_add(rec, "get_gpu_count()")
        return gpu_count

    def get_default_gpu_index():
        _rec_add(rec, "get_default_gpu_index()")
        return default_gpu

    def get_gpu_info(device):
        _rec_add(rec, f"get_gpu_info({device})")
        return _FakeInfo()

    return types.SimpleNamespace(
        Net=net_factory,
        Mat=mat_factory,
        get_gpu_count=get_gpu_count,
        get_default_gpu_index=get_default_gpu_index,
        get_gpu_info=get_gpu_info,
    )


def new_rec():
    return types.SimpleNamespace(entries=[], mats=[])


def net_dirs(tag):
    return scratch(f"{tag}-in"), scratch(f"{tag}-out")


def write_net_files(dir_path):
    param = dir_path / "net.param"
    bin_path = dir_path / "net.bin"
    param.write_text(PARAM)
    bin_path.write_bytes(b"")
    return str(param), str(bin_path)


class _GridEngine:
    def __init__(self, shape):
        self.shape = shape

    def infer(self, chw):
        return np.arange(420000, dtype=np.float32).reshape(self.shape)


class _FlakyEngine:
    def __init__(self, fail_on, short):
        self.fail_on = fail_on
        self.short = short
        self.calls = 0

    def infer(self, chw):
        self.calls += 1
        if self.calls == self.fail_on:
            if self.short:
                return np.zeros(419999, dtype=np.float32)
            raise REFERENCE.RunFailure("device lost")
        return np.zeros(420000, dtype=np.float32)


class _OtherErrorEngine:
    def __init__(self, exc_type, fail_on=2):
        self.exc_type = exc_type
        self.fail_on = fail_on
        self.calls = 0

    def infer(self, chw):
        self.calls += 1
        if self.calls == self.fail_on:
            raise self.exc_type("boom")
        return np.zeros(420000, dtype=np.float32)


class ReferenceTests(unittest.TestCase):
    def test_the_stub_run_writes_the_formula_outputs_and_a_latency_file(self):
        in_dir, out_dir = net_dirs("stub-run")
        write_tensors(in_dir, 2)
        result = run_script(["--in", in_dir, "--out", out_dir, "--stub"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), f"ran 2 tensors into {out_dir}\n")
        for k in range(2):
            want = expected_values(np.frombuffer(tensor_bytes(k), dtype=np.uint8))
            got_path = out_dir / f"output-0000{k}.f32"
            raw = got_path.read_bytes()
            self.assertEqual(len(raw), 1680000)
            self.assertEqual(raw, want.astype("<f4").tobytes())
        lines = (out_dir / "latency.csv").read_text().splitlines()
        self.assertEqual(lines[0], "index,microseconds")
        self.assertEqual(len(lines), 3)
        for k in range(2):
            index, microseconds = lines[k + 1].split(",")
            self.assertEqual(index, str(k))
            self.assertGreater(int(microseconds), 0)
        self.assertEqual(
            held_names(out_dir),
            ["latency.csv", "output-00000.f32", "output-00001.f32"],
        )

    def test_the_files_are_readable_by_the_rust_compare_and_a_perturbation_is_seen(self):
        in_dir, out_a = net_dirs("compare")
        write_tensors(in_dir, 2)
        result = run_script(["--in", in_dir, "--out", out_a, "--stub"])
        self.assertEqual(result.returncode, 0, result.stderr)
        out_b = scratch("compare-b")
        for k in range(2):
            want = expected_values(np.frombuffer(tensor_bytes(k), dtype=np.uint8))
            (out_b / f"output-0000{k}.f32").write_bytes(want.astype("<f4").tobytes())
        (out_b / "latency.csv").write_text("index,microseconds\n0,1\n1,2\n")
        same = run_tool(["compare", "--a", out_a, "--b", out_b])
        self.assertEqual(same.returncode, 0, same.stdout + same.stderr)
        text = same.stdout.decode()
        self.assertIn("max_abs_error 0\n", text)
        self.assertIn("first_difference none\n", text)
        out_c = scratch("compare-c")
        shutil.copytree(out_b, out_c / "b", dirs_exist_ok=True)
        copy_dir = out_c / "b"
        patched = bytearray((copy_dir / "output-00000.f32").read_bytes())
        patched[0:4] = np.float32(0.5).astype("<f4").tobytes()
        (copy_dir / "output-00000.f32").write_bytes(bytes(patched))
        differ = run_tool(["compare", "--a", out_a, "--b", copy_dir])
        self.assertEqual(differ.returncode, 1, differ.stdout + differ.stderr)
        text = differ.stdout.decode()
        self.assertIn("max_abs_error 0.5\n", text)
        self.assertIn("first_difference tensor 0 index 0 a 0 b 0.5\n", text)

    def test_the_ncnn_engine_makes_the_deployed_calls_in_order(self):
        net_dir = scratch("calls")
        param_path, bin_path = write_net_files(net_dir)
        rec = new_rec()
        engine = REFERENCE.NcnnEngine(fake_ncnn(rec), param_path, None, False)
        expected = [
            "Net()",
            "use_vulkan_compute = True",
            "get_gpu_count()",
            "get_default_gpu_index()",
            "get_gpu_info(0)",
            "set_vulkan_device(0)",
            "use_fp16_packed = False",
            "use_fp16_storage = False",
            "use_fp16_arithmetic = False",
            f"load_param({param_path})",
            f"load_model({bin_path})",
            "Mat()",
            "create_extractor()",
            "input(in0)",
            "extract(out0)",
        ]
        self.assertEqual(rec.entries, expected)
        warmup = rec.mats[0]
        self.assertEqual(warmup.shape, (3, 640, 640))
        self.assertTrue(np.all(warmup == 0))
        chw = np.zeros((3, 640, 640), dtype=np.float32)
        engine.infer(chw)
        self.assertEqual(
            rec.entries[len(expected):],
            ["Mat()", "create_extractor()", "input(in0)", "extract(out0)"],
        )
        cpu_rec = new_rec()
        REFERENCE.NcnnEngine(fake_ncnn(cpu_rec), param_path, None, True)
        self.assertIn("use_vulkan_compute = False", cpu_rec.entries)
        for forbidden in ("get_gpu_count()", "set_vulkan_device(0)", "get_gpu_info(0)"):
            self.assertNotIn(forbidden, cpu_rec.entries)
        tight_rec = new_rec()
        with self.assertRaises(REFERENCE.RunFailure):
            REFERENCE.NcnnEngine(fake_ncnn(tight_rec, gpu_count=1), param_path, 1, False)

    def test_engine_failures_are_failures_and_a_failed_run_leaves_nothing_behind(self):
        net_dir = scratch("engine-failures")
        param_path, _ = write_net_files(net_dir)
        rec = new_rec()
        with self.assertRaises(REFERENCE.RunFailure):
            REFERENCE.NcnnEngine(fake_ncnn(rec, param_rc=-1), param_path, None, False)
        rec = new_rec()
        with self.assertRaises(REFERENCE.RunFailure):
            REFERENCE.NcnnEngine(fake_ncnn(rec, model_rc=-1), param_path, None, False)
        rec = new_rec()
        with self.assertRaises(REFERENCE.RunFailure) as caught:
            REFERENCE.NcnnEngine(fake_ncnn(rec, extract_rc=-1), param_path, None, False)
        self.assertIn("-1", str(caught.exception))
        for case in ("raises", "short"):
            in_dir, out_dir = net_dirs(f"engine-fail-{case}")
            write_tensors(in_dir, 2)
            engine = _FlakyEngine(2, case == "short")
            with self.assertRaises(REFERENCE.RunFailure):
                REFERENCE.run(engine, str(in_dir), str(out_dir))
            leftovers = [
                name
                for name in held_names(out_dir)
                if (name.startswith("output-") and name.endswith(".f32")) or name == "latency.csv"
            ]
            self.assertEqual(leftovers, [])

    def assert_refused(self, result):
        self.assertEqual(result.returncode, 3)
        self.assertEqual(result.stdout, b"")
        text = result.stderr.decode()
        self.assertEqual(text.count("\n"), 1)
        self.assertTrue(text.startswith("error:"))

    def test_unusable_inputs_and_outputs_are_refused_before_any_engine_work(self):
        empty_in = scratch("refuse-empty-in")
        out = scratch("refuse-empty-out")
        result = run_script(["--in", empty_in, "--out", out, "--stub"])
        self.assert_refused(result)
        self.assertEqual(list(out.iterdir()), [])

        gap_in, gap_out = net_dirs("refuse-gap")
        good = tensor_bytes(0)
        (gap_in / "tensor-00000.rgb").write_bytes(good)
        (gap_in / "tensor-00002.rgb").write_bytes(good)
        result = run_script(["--in", gap_in, "--out", gap_out, "--stub"])
        self.assert_refused(result)
        self.assertEqual(list(gap_out.iterdir()), [])

        short_in, short_out = net_dirs("refuse-short")
        (short_in / "tensor-00000.rgb").write_bytes(good[:-1])
        result = run_script(["--in", short_in, "--out", short_out, "--stub"])
        self.assert_refused(result)
        self.assertEqual(list(short_out.iterdir()), [])

        held_in, held_out = net_dirs("refuse-held-out")
        (held_in / "tensor-00000.rgb").write_bytes(good)
        (held_out / "output-00000.f32").write_bytes(b"keep")
        result = run_script(["--in", held_in, "--out", held_out, "--stub"])
        self.assert_refused(result)
        self.assertEqual((held_out / "output-00000.f32").read_bytes(), b"keep")
        self.assertEqual(held_names(held_out), ["output-00000.f32"])

        late_in, late_out = net_dirs("refuse-latency-out")
        (late_in / "tensor-00000.rgb").write_bytes(good)
        (late_out / "latency.csv").write_text("index,microseconds\n0,1\n")
        result = run_script(["--in", late_in, "--out", late_out, "--stub"])
        self.assert_refused(result)
        self.assertEqual(held_names(late_out), ["latency.csv"])

        param_in, param_out = net_dirs("refuse-param")
        (param_in / "tensor-00000.rgb").write_bytes(good)
        result = run_script(["--in", param_in, "--out", param_out, "--param", f"{param_out}/missing.param"])
        self.assert_refused(result)
        self.assertEqual(list(param_out.iterdir()), [])

        extra_in, extra_out = net_dirs("contrast-extra")
        (extra_in / "tensor-00000.rgb").write_bytes(good)
        (extra_in / "tensor-00000.rgb.bak").write_bytes(b"not a tensor file")
        result = run_script(["--in", extra_in, "--out", extra_out, "--stub"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), f"ran 1 tensors into {extra_out}\n")

    def test_usage_errors_exit_2(self):
        cases = [
            ["--in", "i", "--out", "o"],
            ["--in", "i", "--out", "o", "--stub", "--param", "x.param"],
            ["--stub", "--out", "o"],
            ["--stub", "--in", "i"],
            ["--in", "i", "--out", "o", "--stub", "--cpu"],
            ["--in", "i", "--out", "o", "--stub", "--device", "0"],
            ["--in", "i", "--out", "o", "--param", "x.param", "--device", "0", "--cpu"],
            ["--in", "i", "--out", "o", "--stub", "--device", "-1"],
            ["--in", "i", "--out", "o", "--param", "x.param", "--device", "-1"],
            ["--in", "i", "--out", "o", "--stub", "--bogus"],
        ]
        for case in cases:
            result = run_script(case)
            self.assertEqual(result.returncode, 2, case)
            self.assertEqual(result.stdout, b"", case)
            self.assertIn(b"usage:", result.stderr, case)

    def test_blob_names_follow_the_first_input_and_the_last_layer(self):
        net_dir = scratch("blobs")
        good = net_dir / "good.param"
        good.write_text(
            "7767517\n"
            "Input data 0 1 data 0=640 1=640 2=3\n"
            "Flatten mid 1 1 data mid0\n"
            "Reshape last 1 1 mid0 result second 0=8400 1=50 2=1\n"
        )
        self.assertEqual(REFERENCE.parse_blob_names(str(good)), ("data", "result"))
        two_inputs = net_dir / "two_inputs.param"
        two_inputs.write_text(
            "7767517\n"
            "Input data 0 1 data 0=640 1=640 2=3\n"
            "Input data2 0 1 data2 0=640 1=640 2=3\n"
            "Flatten mid 1 1 data2 mid0\n"
            "Reshape last 1 1 mid0 result second 0=8400 1=50 2=1\n"
        )
        self.assertEqual(REFERENCE.parse_blob_names(str(two_inputs)), ("data", "result"))
        no_input = net_dir / "no_input.param"
        no_input.write_text(
            "7767517\n"
            "Flatten mid 0 1 none mid0\n"
            "Reshape last 1 1 mid0 result 0=8400 1=50 2=1\n"
        )
        with self.assertRaises(REFERENCE.RunFailure):
            REFERENCE.parse_blob_names(str(no_input))
        empty = net_dir / "empty.param"
        empty.write_text("")
        with self.assertRaises(REFERENCE.RunFailure):
            REFERENCE.parse_blob_names(str(empty))

    def test_a_two_dimensional_result_is_written_in_row_major_order(self):
        in_dir = scratch("row-major-in")
        write_tensors(in_dir, 1)
        for shape in ((50, 8400), (8400, 50)):
            out_dir = scratch(f"row-major-{shape[0]}x{shape[1]}")
            count = REFERENCE.run(_GridEngine(shape), str(in_dir), str(out_dir))
            self.assertEqual(count, 1)
            raw = (out_dir / "output-00000.f32").read_bytes()
            self.assertEqual(len(raw), 1680000)
            np.testing.assert_array_equal(
                np.frombuffer(raw, dtype="<f4"),
                np.arange(420000, dtype="<f4"),
            )

    def test_any_failure_leaves_nothing_behind(self):
        for exc_type in (ValueError, KeyboardInterrupt):
            in_dir, out_dir = net_dirs(f"other-failure-{exc_type.__name__}")
            write_tensors(in_dir, 2)
            with self.assertRaises(exc_type):
                REFERENCE.run(_OtherErrorEngine(exc_type), str(in_dir), str(out_dir))
            leftovers = [
                name
                for name in held_names(out_dir)
                if (name.startswith("output-") and name.endswith(".f32")) or name == "latency.csv"
            ]
            self.assertEqual(leftovers, [])

    def test_a_tensor_that_changes_size_after_the_check_is_a_failure(self):
        in_dir, out_dir = net_dirs("changed-size")
        write_tensors(in_dir, 2)
        second = in_dir / "tensor-00001.rgb"
        with mock.patch.object(REFERENCE, "_read_bytes", side_effect=[tensor_bytes(0), b"0" * 10]):
            with self.assertRaises(REFERENCE.RunFailure) as caught:
                REFERENCE.run(REFERENCE.StubEngine(), str(in_dir), str(out_dir))
        text = str(caught.exception)
        self.assertIn(str(second), text)
        self.assertIn("10", text)
        self.assertIn("1228800", text)
        self.assertEqual(held_names(out_dir), [])

    def test_a_file_that_cannot_be_removed_is_named_in_the_failure(self):
        in_dir, out_dir = net_dirs("uncleanable")
        write_tensors(in_dir, 3)
        real_remove = REFERENCE.os.remove

        def refusing_remove(path, *args, **kwargs):
            if str(path).endswith("output-00000.f32"):
                raise PermissionError(13, "refused by the test")
            return real_remove(path, *args, **kwargs)

        with mock.patch.object(REFERENCE.os, "remove", refusing_remove):
            with self.assertRaises(REFERENCE.RunFailure) as caught:
                REFERENCE.run(_FlakyEngine(3, False), str(in_dir), str(out_dir))
        text = str(caught.exception)
        self.assertIn("could not remove", text)
        self.assertIn("output-00000.f32", text)
        self.assertNotIn("output-00001.f32", text)
        self.assertIsInstance(caught.exception.__context__, REFERENCE.RunFailure)
        self.assertIn("device lost", str(caught.exception.__context__))
        self.assertFalse((out_dir / "output-00001.f32").exists())
        self.assertTrue((out_dir / "output-00000.f32").exists())
        REFERENCE.os.remove(out_dir / "output-00000.f32")

    def test_validation_comes_before_the_engine_is_built(self):
        net_dir = scratch("precheck-net")
        (net_dir / "ok.param").write_text(PARAM)
        (net_dir / "ok.bin").write_bytes(b"")
        empty_in = scratch("precheck-empty-in")
        out = scratch("precheck-out")
        result = run_script(["--in", empty_in, "--out", out, "--param", str(net_dir / "ok.param")])
        self.assert_refused(result)
        self.assertIn("no tensor files", result.stderr.decode())
        self.assertNotIn("ncnn", result.stderr.decode())
        in_dir = scratch("precheck-in")
        write_tensors(in_dir, 1)
        held_out = scratch("precheck-held")
        (held_out / "output-00000.f32").write_bytes(b"keep")
        result = run_script(["--in", in_dir, "--out", held_out, "--param", str(net_dir / "ok.param")])
        self.assert_refused(result)
        self.assertNotIn("ncnn", result.stderr.decode())
        self.assertEqual((held_out / "output-00000.f32").read_bytes(), b"keep")


if __name__ == "__main__":
    unittest.main()
