"""Goldens for corvette-yolo's normalize: the float32 bits of Frigate's input transform.

The deployed detector asks for nchw and float, so DetectorStandIn supplies exactly
those two values and its _transform_input body applies the quoted lines unchanged.

Reference frigate/object_detection/base.py at v0.17.2,
blob d2a54afbc570220846207dde13713307013b93d9:

    def _transform_input(self, tensor_input: np.ndarray) -> np.ndarray:
        if self.input_transform:
            tensor_input = np.transpose(tensor_input, self.input_transform)

        if self.dtype == InputDTypeEnum.float:
            tensor_input = tensor_input.astype(np.float32)
            tensor_input /= 255
        elif self.dtype == InputDTypeEnum.float_denorm:
            tensor_input = tensor_input.astype(np.float32)

        return tensor_input

Reference frigate/object_detection/util.py at v0.17.2,
blob ea8bd4226ce03641b471e58c3f9775e3e827b470:

    def tensor_transform(desired_shape: InputTensorEnum):
        # Currently this function only supports BHWC permutations
        if desired_shape == InputTensorEnum.nhwc:
            return None
        elif desired_shape == InputTensorEnum.nchw:
            return (0, 3, 1, 2)
        elif desired_shape == InputTensorEnum.hwnc:
            return (1, 2, 0, 3)
        elif desired_shape == InputTensorEnum.hwcn:
            return (1, 2, 3, 0)
"""

from pathlib import Path

import numpy as np

SIZE = 640
CHANNELS = 3
FIXTURES = Path(__file__).resolve().parents[2] / "crates" / "corvette-yolo" / "tests" / "fixtures"


class InputDTypeEnum:
    float = "float"
    float_denorm = "float_denorm"


class InputTensorEnum:
    nhwc = "nhwc"
    nchw = "nchw"
    hwnc = "hwnc"
    hwcn = "hwcn"


def tensor_transform(desired_shape):
    # Currently this function only supports BHWC permutations
    if desired_shape == InputTensorEnum.nhwc:
        return None
    elif desired_shape == InputTensorEnum.nchw:
        return (0, 3, 1, 2)
    elif desired_shape == InputTensorEnum.hwnc:
        return (1, 2, 0, 3)
    elif desired_shape == InputTensorEnum.hwcn:
        return (1, 2, 3, 0)


class DetectorStandIn:
    def __init__(self):
        self.input_transform = tensor_transform(InputTensorEnum.nchw)
        self.dtype = InputDTypeEnum.float

    def _transform_input(self, tensor_input: np.ndarray) -> np.ndarray:
        if self.input_transform:
            tensor_input = np.transpose(tensor_input, self.input_transform)

        if self.dtype == InputDTypeEnum.float:
            tensor_input = tensor_input.astype(np.float32)
            tensor_input /= 255
        elif self.dtype == InputDTypeEnum.float_denorm:
            tensor_input = tensor_input.astype(np.float32)

        return tensor_input


def hex_bits(transformed, index):
    return f"0x{int(transformed.view(np.uint32)[index]):08x}"


def table_lines(detector):
    lines = []
    for v in range(256):
        one_pixel = np.array([[[[v, v, v]]]], dtype=np.uint8)
        lines.append(f"{v} {hex_bits(detector._transform_input(one_pixel), (0, 0, 0, 0))}")
    return lines


def spot_positions():
    for c in range(CHANNELS):
        for y in (0, SIZE - 1):
            for x in (0, SIZE - 1):
                yield c, y, x
    for k in range(600):
        yield k % 3, (k * 89) % SIZE, (k * 131 + 7) % SIZE


def spot_lines(detector):
    grid = np.fromfunction(
        lambda y, x, c: (y * 7 + x * 13 + c * 31) % 256,
        (SIZE, SIZE, CHANNELS),
        dtype=np.int64,
    )
    image = grid.astype(np.uint8)[np.newaxis]
    transformed = detector._transform_input(image)
    return [f"{c} {y} {x} {hex_bits(transformed, (0, c, y, x))}" for c, y, x in spot_positions()]


def main():
    detector = DetectorStandIn()
    FIXTURES.mkdir(parents=True, exist_ok=True)
    table = table_lines(detector)
    spots = spot_lines(detector)
    (FIXTURES / "normalize_table.txt").write_text("\n".join(table) + "\n")
    (FIXTURES / "normalize_spots.txt").write_text("\n".join(spots) + "\n")
    print(f"wrote {len(table)} table lines and {len(spots)} spot lines")


if __name__ == "__main__":
    main()
