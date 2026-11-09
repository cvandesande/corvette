"""Run a Corvette tensor set through an engine and write the raw outputs for compare.

NcnnEngine makes the same ncnn calls as the deployed Python detector; StubEngine
answers with the first RAW_VALUES of the flattened input, so --stub and the
module import work with numpy and the standard library alone.
A RunFailure prints one error: line and exits 3; any other exception exits 1.
"""

import argparse
import os
import sys
import time

import numpy as np

TENSOR_BYTES = 640 * 640 * 3
RAW_VALUES = 50 * 8400


class RunFailure(Exception):
    """The run failed; the caller prints one error: line and exits 3."""


def parse_blob_names(path):
    """Return the first Input layer's output blob and the final layer's first output blob."""
    layers = []
    try:
        handle = open(path, encoding="utf-8")
    except OSError as exc:
        raise RunFailure(f"cannot read {path}: {exc}") from exc
    with handle:
        for raw_line in handle:
            fields = raw_line.split()
            if len(fields) < 4 or fields[0].startswith("#"):
                continue
            try:
                input_count = int(fields[2])
                output_count = int(fields[3])
            except ValueError:
                continue
            blob_end = 4 + input_count + output_count
            if output_count and len(fields) >= blob_end:
                layers.append(fields[:blob_end])
    if not layers:
        raise RunFailure(f"no layers found in parameter file {path}")
    input_layers = [layer for layer in layers if layer[0] == "Input"]
    if not input_layers:
        raise RunFailure(f"no Input layer found in {path}")
    first_input = input_layers[0]
    last_layer = layers[-1]
    return (first_input[4 + int(first_input[2])], last_layer[4 + int(last_layer[2])])


class StubEngine:
    """Answer with the first RAW_VALUES of the flattened (3, 640, 640) input."""

    def infer(self, chw):
        flat = np.ascontiguousarray(chw, dtype=np.float32).reshape(-1)
        return flat[:RAW_VALUES].copy()


class NcnnEngine:
    """The deployed detector's ncnn sequence: construct loads and warms up, infer runs."""

    def __init__(self, ncnn_module, param_path, device, cpu):
        self.ncnn = ncnn_module
        self.net = ncnn_module.Net()
        self.net.opt.use_vulkan_compute = not cpu
        if not cpu:
            gpu_count = ncnn_module.get_gpu_count()
            if device is None:
                device = ncnn_module.get_default_gpu_index()
            if not 0 <= device < gpu_count:
                raise RunFailure(
                    f"device {device} out of range, {gpu_count} Vulkan device(s) present"
                )
            print(
                f"device {device}: {ncnn_module.get_gpu_info(device).device_name()}",
                file=sys.stderr,
            )
            self.net.set_vulkan_device(device)
        # A Vulkan build picks fp16 paths by default; parity needs a real fp32 run.
        self.net.opt.use_fp16_packed = False
        self.net.opt.use_fp16_storage = False
        self.net.opt.use_fp16_arithmetic = False
        if self.net.load_param(param_path) != 0:
            raise RunFailure(f"failed to parse parameter file {param_path}")
        bin_path = param_path.removesuffix(".param") + ".bin"
        if self.net.load_model(bin_path) != 0:
            raise RunFailure(f"failed to load weights {bin_path}")
        self.input_name, self.output_name = parse_blob_names(param_path)
        # A weight upload that faulted the device only shows at the first run.
        self.infer(np.zeros((3, 640, 640), dtype=np.float32))

    def infer(self, chw):
        # ncnn.Mat borrows chw's buffer, so chw stays referenced through extract.
        mat = self.ncnn.Mat(chw)
        extractor = self.net.create_extractor()
        if extractor.input(self.input_name, mat) != 0:
            raise RunFailure(f"ncnn input {self.input_name} was refused")
        result, output = extractor.extract(self.output_name)
        if result != 0:
            raise RunFailure(f"ncnn extract {self.output_name} returned {result}")
        return np.array(output, dtype=np.float32)


def _digits(name, prefix, suffix):
    if not (name.startswith(prefix) and name.endswith(suffix)):
        return None
    middle = name[len(prefix): len(name) - len(suffix)]
    if len(middle) == 5 and middle.isascii() and middle.isdigit():
        return int(middle)
    return None


def _read_bytes(path):
    try:
        with open(path, "rb") as handle:
            return handle.read()
    except OSError as exc:
        raise RunFailure(f"cannot read {path}: {exc}") from exc


def _write_bytes(path, data):
    try:
        with open(path, "wb") as handle:
            handle.write(data)
    except OSError as exc:
        raise RunFailure(f"cannot write {path}: {exc}") from exc


def _tensor_paths(in_dir):
    try:
        names = os.listdir(in_dir)
    except OSError as exc:
        raise RunFailure(f"cannot list {in_dir}: {exc}") from exc
    numbers = sorted(
        number
        for number in (_digits(name, "tensor-", ".rgb") for name in names)
        if number is not None
    )
    if not numbers:
        raise RunFailure(f"{in_dir} holds no tensor files")
    for position, number in enumerate(numbers):
        if number != position:
            raise RunFailure(f"{in_dir} has a gap or an extra file in the tensor numbering")
    paths = []
    for number in numbers:
        path = os.path.join(in_dir, f"tensor-{number:05}.rgb")
        try:
            size = os.stat(path).st_size
        except OSError as exc:
            raise RunFailure(f"cannot inspect {path}: {exc}") from exc
        if size != TENSOR_BYTES:
            raise RunFailure(f"{path} is {size} bytes, expected {TENSOR_BYTES}")
        paths.append(path)
    return paths


def _refuse_existing_out(out_dir):
    if not os.path.isdir(out_dir):
        return
    try:
        names = os.listdir(out_dir)
    except OSError as exc:
        raise RunFailure(f"cannot list {out_dir}: {exc}") from exc
    for name in names:
        if name == "latency.csv" or _digits(name, "output-", ".f32") is not None:
            raise RunFailure(f"{out_dir} already holds output files or a latency file")


def check_inputs(in_dir, out_dir):
    paths = _tensor_paths(in_dir)
    _refuse_existing_out(out_dir)
    return paths


class _Cleanup:
    """Remove every file a failing run registered, including a half-written one."""

    def __init__(self, out_dir):
        self.out_dir = out_dir
        self.registered = []

    def register(self, path):
        self.registered.append(path)

    def remove_all(self):
        leftovers = []
        paths = list(self.registered) + [os.path.join(self.out_dir, "latency.csv")]
        for path in paths:
            try:
                os.remove(path)
            except FileNotFoundError:
                pass
            except OSError:
                leftovers.append(path)
        return sorted(leftovers)


def run(engine, in_dir, out_dir):
    paths = check_inputs(in_dir, out_dir)
    try:
        os.makedirs(out_dir, exist_ok=True)
    except OSError as exc:
        raise RunFailure(f"cannot create {out_dir}: {exc}") from exc
    cleanup = _Cleanup(out_dir)
    latencies = []
    complete = False
    try:
        for index, path in enumerate(paths):
            raw = _read_bytes(path)
            if len(raw) != TENSOR_BYTES:
                raise RunFailure(f"{path} is {len(raw)} bytes, expected {TENSOR_BYTES}")
            hwc = np.frombuffer(raw, dtype=np.uint8).reshape(640, 640, 3)
            chw = np.transpose(hwc, (2, 0, 1)).astype(np.float32) / np.float32(255.0)
            started = time.perf_counter_ns()
            result = engine.infer(chw)
            microseconds = (time.perf_counter_ns() - started) // 1000
            values = np.ascontiguousarray(result, dtype="<f4").reshape(-1)
            if values.size != RAW_VALUES:
                raise RunFailure(
                    f"tensor {index}: output has {values.size} values, expected {RAW_VALUES}"
                )
            latencies.append(microseconds)
            destination = os.path.join(out_dir, f"output-{index:05}.f32")
            cleanup.register(destination)
            _write_bytes(destination, values.tobytes())
        latency_path = os.path.join(out_dir, "latency.csv")
        content = "index,microseconds\n" + "".join(
            f"{index},{microseconds}\n" for index, microseconds in enumerate(latencies)
        )
        cleanup.register(latency_path)
        _write_bytes(latency_path, content.encode("utf-8"))
        complete = True
    finally:
        if not complete:
            leftovers = cleanup.remove_all()
            if leftovers:
                raise RunFailure(
                    f"could not remove {', '.join(leftovers)} after a failed run"
                )
    return len(paths)


def _nonneg_int(text):
    try:
        value = int(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"invalid integer: {text}") from None
    if value < 0:
        raise argparse.ArgumentTypeError(f"invalid device index: {text}")
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="python_reference.py",
        usage="%(prog)s --in DIR --out DIR (--stub | --param PATH [--device N | --cpu])",
    )
    parser.add_argument("--in", dest="in_dir", required=True, metavar="DIR")
    parser.add_argument("--out", dest="out", required=True, metavar="DIR")
    engines = parser.add_mutually_exclusive_group(required=True)
    engines.add_argument("--stub", action="store_true")
    engines.add_argument("--param", metavar="PATH")
    devices = parser.add_mutually_exclusive_group()
    devices.add_argument("--device", type=_nonneg_int, metavar="N")
    devices.add_argument("--cpu", action="store_true")
    args = parser.parse_args(argv)
    if not args.param and (args.cpu or args.device is not None):
        parser.error("--cpu and --device are valid only with --param")
    try:
        check_inputs(args.in_dir, args.out)
        if args.stub:
            engine = StubEngine()
        else:
            param_path = args.param
            if not param_path.endswith(".param"):
                raise RunFailure(f"{param_path} is not an ncnn .param path")
            bin_path = param_path.removesuffix(".param") + ".bin"
            for path in (param_path, bin_path):
                if not os.path.isfile(path):
                    raise RunFailure(f"model file not found: {path}")
            try:
                import ncnn
            except ImportError as exc:
                raise RunFailure(f"cannot import ncnn: {exc}") from exc
            engine = NcnnEngine(ncnn, param_path, args.device, args.cpu)
        count = run(engine, args.in_dir, args.out)
    except RunFailure as exc:
        line = f"error: {exc}"
        if exc.__context__ is not None:
            line += f" (first failure: {exc.__context__})"
        print(line, file=sys.stderr)
        return 3
    print(f"ran {count} tensors into {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
