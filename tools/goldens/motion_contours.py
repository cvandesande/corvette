r"""Goldens for corvette-motion's outer contours: for a black and white mask, the
list cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE) returns,
reduced per contour to its cv2.boundingRect and twice its cv2.contourArea, in
the returned order.

The numbers are what the pinned wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256
0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b) writes; the
script prints the wheel's version and its version-control line and refuses to
write the fixture unless the version is 4.11.0. Its only argument is the
directory the wheel was extracted into, and the wheel's native libraries need
two loader directories, so the full run command is:

    nix develop .#goldens --command env \
        LD_LIBRARY_PATH=/nix/store/6s4wm3zl2y0v5g998n2g88k1zh3vf2pr-zlib-1.3.2/lib:/nix/store/0ba5kj6r0zhfwd84l5vzccqvx7amlq2c-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_contours.py <extracted wheel directory>

Measured with the wheel: the contour count, the list order, each contour's
cv2.boundingRect and cv2.contourArea. Read from source, not measured: the
padding, binarisation, offset, hole rule, corner-point emission and area
formula quoted below. cv::boundingRect is not among the fetched files; its rule
(the pixel extent, max minus min plus one) is established by this fixture
itself. Contour tracing has no CPU-specific code path: that is a reading of the
source of contours_new.cpp, not a measurement across CPUs.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 131 to 146,
copied verbatim:

        # dilate the thresholded image to fill in holes, then find contours
        # on thresholded image
        thresh_dilated = cv2.dilate(thresh, None, iterations=1)
        contours = cv2.findContours(
            thresh_dilated, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE
        )
        contours = grab_cv2_contours(contours)

        # loop over the contours
        total_contour_area = 0
        for c in contours:
            # if the contour is big enough, count it as motion
            contour_area = cv2.contourArea(c)
            total_contour_area += contour_area
            if contour_area > self.config.contour_area:
                x, y, w, h = cv2.boundingRect(c)

Reference: the opencv repository, path modules/imgproc/src/contours_new.cpp,
tag 4.11.0, blob 9f8c716fbb61b7434cc0bf2e5fa2bb875ef0b5e1: line 672 (one border
pixel of zero), lines 673 to 674 (every non-zero byte binarised to 1), line 677
(the coordinates offset back by minus one), line 446 (RETR_EXTERNAL skips a
contour that is a hole or lies inside one), and lines 237 to 239 (a point is
kept only where the trace direction changes), copied verbatim:

    copyMakeBorder(_image, image, 1, 1, 1, 1, BORDER_CONSTANT | BORDER_ISOLATED, Scalar(0));
    if (image.type() != CV_32SC1)
        threshold(image, image, 0, 1, THRESH_BINARY);
    ContourScanner scanner = ContourScanner_::create(image, mode, method, offset + Point(-1, -1));
    if (mode == RETR_EXTERNAL && (is_hole || this->image.at<schar>(last_pos) > 0))
            else if (s != prev_s || isDirect)
            {
                res_contour.pts.push_back(pt);

Reference: the opencv repository, path modules/imgproc/src/shapedescr.cpp, tag
4.11.0, blob 007bf9ac62b137206c36ee3aaa58df05c3b72ab2, lines 308 to 338 (the
contour area: a shoelace sum of the point list, times one half, absolute
value), copied verbatim:

double cv::contourArea( InputArray _contour, bool oriented )
{
    CV_INSTRUMENT_REGION();

    Mat contour = _contour.getMat();
    int npoints = contour.checkVector(2);
    int depth = contour.depth();
    CV_Assert(npoints >= 0 && (depth == CV_32F || depth == CV_32S));

    if( npoints == 0 )
        return 0.;

    double a00 = 0;
    bool is_float = depth == CV_32F;
    const Point* ptsi = contour.ptr<Point>();
    const Point2f* ptsf = contour.ptr<Point2f>();
    Point2f prev = is_float ? ptsf[npoints-1] : Point2f((float)ptsi[npoints-1].x, (float)ptsi[npoints-1].y);

    for( int i = 0; i < npoints; i++ )
    {
        Point2f p = is_float ? ptsf[i] : Point2f((float)ptsi[i].x, (float)ptsi[i].y);
        a00 += (double)prev.x * p.y - (double)prev.y * p.x;
        prev = p;
    }

    a00 *= 0.5;
    if( !oriented )
        a00 = fabs(a00);

    return a00;
}

Writes the fixture read by crates/corvette-motion/tests/contours.rs: a first
line "counts hand N1 noise N2 dilated N3 nested N4 small N5 heights N6 levels
N7" with the number of records of each kind, then one line per record, the
kinds in that order:

"contours <kind> <width> <height> <level> <runs> <count> <x> <y> <w> <h>
twice_area ...": <level> is the foreground byte (255 except for levels), <runs>
is the picture as comma-separated decimal run lengths, alternating, the first
run background (it may be 0), summing to width * height row by row; <count> is
followed by count groups of five numbers, the contours in the order the wheel
returned them: cv2.boundingRect as x y w h and twice_area = 2 *
cv2.contourArea, written as an integer after the script asserted it is a whole
number.

Kinds: hand is 28 built-by-name pictures; noise is 12 records of 100 rows by
177 columns (the deployed motion frame of a 1280x720 detect frame) with
foreground where a seeded random is below densities 0.02, 0.1, 0.3, 0.6, three
each; dilated is 32 records of the same size with sparse noise at densities
0.005, 0.01, 0.02, 0.05 dilated once with cv2.dilate(mask, None,
iterations=1), eight each; nested is 20 records of the same size holding three
to six one-pixel-thick rectangular rings of sides 6 to 40, each with a smaller
ring or a blob in its hole, plus dilated sparse noise outside; small is 80
records with each side drawn from 1 to 40 and density from 0.15, 0.3, 0.5, 0.7,
one record in three dilated once; heights is 10 records of the motion frames
of a 1280x720 detect frame with frame heights 50 and 150 (50 rows by 88
columns and 150 rows by 266 columns), dilated sparse noise at density 0.02,
five each; levels is 6 records of 30 rows by 40 columns at density 0.3 with
foreground byte 1, 7 and 128, two each (the wheel treats every non-zero byte
as foreground). Each record draws its numbers from
random.Random(20261110 + 1000 * kind_number + index), the kind number being
the kind's position in the header, from 0.
"""

import random
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/contours_cases.txt"

NOISE = (100, 177)
SMALL_DENSITY = [0.15, 0.3, 0.5, 0.7]
LIMIT = 600_000


def noisy(rng, density, height, width, level=255):
    mask = np.zeros((height, width), np.uint8)
    for i in range(height * width):
        if rng.random() < density:
            mask[i // width, i % width] = level
    return mask


def picture(rows, level=255):
    height, width = len(rows), len(rows[0])
    mask = np.zeros((height, width), np.uint8)
    for y, row in enumerate(rows):
        assert len(row) == width, rows
        for x, char in enumerate(row):
            if char == "#":
                mask[y, x] = level
    return mask


def ring(mask, y0, x0, height, width, level=255):
    mask[y0 : y0 + height, x0 : x0 + width] = level
    mask[y0 + 1 : y0 + height - 1, x0 + 1 : x0 + width - 1] = 0


def blob(mask, y0, x0, height, width, level=255):
    mask[y0 : y0 + height, x0 : x0 + width] = level


def hand_pictures():
    return [
        ("rect5x4", ["#####", "#####", "#####", "#####"]),
        ("isolated", ["...", ".#.", "..."]),
        ("diagonal", ["#..", ".#.", "..#"]),
        (
            "three_blobs",
            ["#.......", "........", "......##", "......##", "........", "##......"],
        ),
        ("open_ring", [".....", ".###.", ".#.#.", ".##..", "....."]),
        (
            "ring_with_blob",
            [".......", "..###..", ".#...#.", ".#.#.#.", ".#...#.", "..###..", "......."],
        ),
        (
            "diamond_ring",
            ["....#....", "...#.#...", "..#...#..", ".#..#..#.", "..#...#..", "...#.#...", "....#...."],
        ),
        (
            "ring_in_ring",
            [
                "###########",
                "#.........#",
                "#.#######.#",
                "#.#.....#.#",
                "#.#.###.#.#",
                "#.#.###.#.#",
                "#.#.....#.#",
                "#.#######.#",
                "#.........#",
                "###########",
            ],
        ),
        ("empty", ["...", "...", "..."]),
        ("one_pixel", ["#"]),
        ("full_row", ["########"]),
        ("full_column", ["#", "#", "#", "#", "#", "#", "#", "#"]),
        ("full_12x9", ["############"] * 9),
        ("corner_north_west", ["#.....", "......", "......", "......", "......", "......"]),
        ("corner_north_east", [".....#", "......", "......", "......", "......", "......"]),
        ("corner_south_west", ["......", "......", "......", "......", "......", "#....."]),
        ("corner_south_east", ["......", "......", "......", "......", "......", ".....#"]),
        ("line_horizontal", [".......", ".......", ".#####.", ".......", "......."]),
        ("line_vertical", ["..#..", "..#..", "..#..", "..#..", "..#.."]),
        ("line_diagonal", ["#....", ".#...", "..#..", "...#.", "....#"]),
        ("line_wide", ["....", "####", "####", "...."]),
        ("l_shape", ["#....", "#....", "#....", "#....", "#####"]),
        ("ring_hole", ["#####", "#...#", "#...#", "#...#", "#####"]),
        ("gap_one", ["##...", ".....", "...##"]),
        ("corner_touch", ["##.", ".##", "..#"]),
        ("all_borders", [".....", "..#..", "..#..", "..#..", "..#..", "#####"]),
        ("pinch_hourglass", ["#####", ".#.#.", "..#..", ".#.#.", "#####"]),
        ("pinch_wide", ["###.###", "#.....#", "###.###"]),
    ]


def record(kind, index, mask, level):
    height, width = mask.shape
    flat = mask.reshape(-1)
    runs = []
    run = 0
    foreground = False
    for byte in flat:
        if bool(byte) == foreground:
            run += 1
        else:
            runs.append(run)
            foreground = not foreground
            run = 1
    runs.append(run)
    count = 0
    groups = []
    for contour in cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)[0]:
        x, y, w, h = cv2.boundingRect(contour)
        twice = 2.0 * cv2.contourArea(contour)
        whole = int(twice)
        assert twice == whole, (kind, index, x, y, twice)
        count += 1
        groups += [x, y, w, h, whole]
    fields = [
        "contours",
        kind,
        str(width),
        str(height),
        str(level),
        ",".join(str(run) for run in runs),
        str(count),
    ]
    fields += [str(value) for value in groups]
    return " ".join(fields)


def noise_records():
    lines = []
    for index, density in enumerate([0.02] * 3 + [0.1] * 3 + [0.3] * 3 + [0.6] * 3):
        rng = random.Random(20261110 + 1000 * 1 + index)
        lines.append(record("noise", index, noisy(rng, density, *NOISE), 255))
    return lines


def dilated_records():
    lines = []
    for index in range(32):
        rng = random.Random(20261110 + 1000 * 2 + index)
        density = [0.005, 0.01, 0.02, 0.05][index // 8]
        mask = cv2.dilate(noisy(rng, density, *NOISE), None, iterations=1)
        lines.append(record("dilated", index, mask, 255))
    return lines


def nested_records():
    lines = []
    for index in range(20):
        rng = random.Random(20261110 + 1000 * 3 + index)
        height, width = NOISE
        mask = np.zeros((height, width), np.uint8)
        covered = np.zeros((height, width), bool)
        for _ in range(rng.randint(3, 6)):
            side_y = rng.randint(6, 40)
            side_x = rng.randint(6, 40)
            y0 = rng.randrange(height - side_y + 1)
            x0 = rng.randrange(width - side_x + 1)
            ring(mask, y0, x0, side_y, side_x)
            covered[y0 : y0 + side_y, x0 : x0 + side_x] = True
            if side_y - 2 >= 6 and side_x - 2 >= 6 and rng.random() < 0.5:
                ring(mask, y0 + 2, x0 + 2, side_y - 4, side_x - 4)
            else:
                blob(mask, y0 + 2, x0 + 2, side_y - 4, side_x - 4)
        noise = cv2.dilate(noisy(rng, 0.005, height, width), None, iterations=1)
        mask = mask | np.where(covered, 0, noise).astype(np.uint8)
        lines.append(record("nested", index, mask, 255))
    return lines


def small_records():
    lines = []
    for index in range(80):
        rng = random.Random(20261110 + 1000 * 4 + index)
        height = rng.randint(1, 40)
        width = rng.randint(1, 40)
        mask = noisy(rng, rng.choice(SMALL_DENSITY), height, width)
        if index % 3 == 0:
            mask = cv2.dilate(mask, None, iterations=1)
        lines.append(record("small", index, mask, 255))
    return lines


def heights_records():
    lines = []
    for index in range(10):
        rng = random.Random(20261110 + 1000 * 5 + index)
        height, width = [(50, 88), (150, 266)][index // 5]
        mask = cv2.dilate(noisy(rng, 0.02, height, width), None, iterations=1)
        lines.append(record("heights", index, mask, 255))
    return lines


def levels_records():
    lines = []
    for index in range(6):
        rng = random.Random(20261110 + 1000 * 6 + index)
        level = [1, 7, 128][index // 2]
        lines.append(record("levels", index, noisy(rng, 0.3, 30, 40, level), level))
    return lines


def hand_records():
    lines = []
    for index, (_name, rows) in enumerate(hand_pictures()):
        lines.append(record("hand", index, picture(rows), 255))
    return lines


def main():
    print("cv2.__version__ =", cv2.__version__)
    for line in cv2.getBuildInformation().splitlines():
        if "Version control" in line:
            print(line.strip())
    assert cv2.__version__ == "4.11.0", cv2.__version__
    blocks = {
        "hand": hand_records(),
        "noise": noise_records(),
        "dilated": dilated_records(),
        "nested": nested_records(),
        "small": small_records(),
        "heights": heights_records(),
        "levels": levels_records(),
    }
    kinds = ["hand", "noise", "dilated", "nested", "small", "heights", "levels"]
    while len(text_of(kinds, blocks).encode("ascii")) > LIMIT:
        assert blocks["noise"], "the fixture exceeds the limit without noise records"
        blocks["noise"].pop()
    OUT.write_text(text_of(kinds, blocks), encoding="ascii")


def text_of(kinds, blocks):
    counts = [len(blocks[kind]) for kind in kinds]
    lines = ["counts " + " ".join(f"{kind} {count}" for kind, count in zip(kinds, counts))]
    for kind in kinds:
        lines.extend(blocks[kind])
    return "\n".join(lines) + "\n"


main()
