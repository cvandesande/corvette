//! Draws the box file of motion-detect onto chosen pictures of a raw luma file as PPM files.
use crate::Failure;
use crate::args::Flags;
use crate::motion_detect::{open_input, open_output};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::ExitCode;

const ANNOTATE_FLAGS: &[&str] = &["luma", "width", "height", "boxes", "frames", "out"];
const RED: [u8; 3] = [255, 0, 0];
const PPM_MAX_VALUE: u8 = 255;
fn side(flags: &Flags, name: &str) -> Result<usize, Failure> {
    let Ok(value) = flags.required(name)?.parse::<usize>() else {
        return Err(Failure::Usage);
    };
    (value > 0).then_some(value).ok_or(Failure::Usage)
}
pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = Flags::parse(args, ANNOTATE_FLAGS).ok_or(Failure::Usage)?;
    let (width, height) = (side(&flags, "width")?, side(&flags, "height")?);
    let luma_path = flags.required("luma")?;
    let boxes_path = flags.required("boxes")?;
    let out_dir = flags.required("out")?;
    let raw = flags
        .required("frames")?
        .split(',')
        .map(str::parse::<usize>);
    let parts: Result<Vec<usize>, _> = raw.collect();
    let wanted: BTreeSet<usize> = parts.map_err(|_| Failure::Usage)?.into_iter().collect();
    let pixels = width.checked_mul(height).ok_or(Failure::Usage)?;
    let picture_bytes = u64::try_from(pixels).map_err(|_| Failure::Usage)?;
    let mut luma = open_input(luma_path)?;
    let bytes = luma
        .metadata()
        .map_err(|e| Failure::Input(format!("cannot inspect {luma_path}: {e}")))?
        .len();
    if bytes == 0 || bytes % picture_bytes != 0 {
        let msg = format!("{luma_path} is {bytes} bytes, a picture is {picture_bytes} bytes");
        return Err(Failure::Input(msg));
    }
    let picture_count = usize::try_from(bytes / picture_bytes).map_err(|_| Failure::Usage)?;
    for &number in &wanted {
        if number >= picture_count {
            let msg = format!("picture {number} beyond {picture_count} pictures");
            return Err(Failure::Input(msg));
        }
    }
    let boxes = read_box_lines(boxes_path)?;
    std::fs::metadata(out_dir)
        .ok()
        .filter(std::fs::Metadata::is_dir)
        .ok_or_else(|| Failure::Input(format!("{out_dir} is not a directory")))?;
    for &number in &wanted {
        if !boxes.contains_key(&number) {
            let msg = format!("no line for picture {number} in {boxes_path}");
            return Err(Failure::Input(msg));
        }
    }
    let (mut written, mut drawn) = (0_usize, 0_usize);
    for &number in &wanted {
        let line = boxes.get(&number).ok_or_else(|| {
            Failure::Input(format!("no line for picture {number} in {boxes_path}"))
        })?;
        drawn += line.len();
        let offset = u64::try_from(number).map_err(|_| Failure::Usage)? * picture_bytes;
        luma.seek(SeekFrom::Start(offset))
            .map_err(|e| Failure::Input(format!("cannot seek {number}: {e}")))?;
        let mut picture = vec![0u8; pixels];
        luma.read_exact(&mut picture)
            .map_err(|e| Failure::Input(format!("cannot read {number}: {e}")))?;
        let path = format!("{out_dir}/frame-{number}.ppm");
        let mut file = open_output(&path)?;
        let header = format!("P{}\n{width} {height}\n{PPM_MAX_VALUE}\n", 6);
        file.write_all(header.as_bytes())
            .and_then(|()| file.write_all(&paint_boxes(&picture, width, height, line)))
            .map_err(|e| Failure::Input(format!("cannot write {path}: {e}")))?;
        written += 1;
    }
    writeln!(std::io::stdout(), "frames {written}, boxes drawn {drawn}")
        .map_err(|e| Failure::Input(format!("cannot write the summary: {e}")))?;
    Ok(ExitCode::SUCCESS)
}
fn read_box_lines(path: &str) -> Result<HashMap<usize, Vec<[usize; 4]>>, Failure> {
    let mut reader = BufReader::new(open_input(path)?);
    let mut by_index: HashMap<usize, Vec<[usize; 4]>> = HashMap::new();
    let mut text = String::new();
    let mut number = 0_usize;
    loop {
        text.clear();
        let read = reader
            .read_line(&mut text)
            .map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))?;
        if read == 0 {
            return Ok(by_index);
        }
        number += 1;
        let fields: Vec<&str> = text.trim_end_matches('\n').split('\t').collect();
        if fields.len() != 3 {
            let msg = format!("line {number} of {path}: not three fields");
            return Err(Failure::Input(msg));
        }
        let Ok(index) = fields[0].parse::<usize>() else {
            let msg = format!("line {number} of {path}: bad index");
            return Err(Failure::Input(msg));
        };
        let mut boxes: Vec<[usize; 4]> = Vec::new();
        if fields[2] != "-" {
            for group in fields[2].split(';') {
                let parsed = group
                    .split(',')
                    .map(|part| part.parse::<usize>().ok())
                    .collect::<Option<Vec<usize>>>()
                    .and_then(|numbers| numbers.try_into().ok());
                let Some(parsed) = parsed else {
                    let msg = format!("line {number} of {path}: {group:?} is not x1,y1,x2,y2");
                    return Err(Failure::Input(msg));
                };
                boxes.push(parsed);
            }
        }
        by_index.insert(index, boxes);
    }
}
fn paint_boxes(grey: &[u8], width: usize, height: usize, boxes: &[[usize; 4]]) -> Vec<u8> {
    let mut rgb = vec![0u8; grey.len() * 3];
    for (pixel, grey_pixel) in rgb.chunks_exact_mut(3).zip(grey) {
        pixel.copy_from_slice(&[*grey_pixel, *grey_pixel, *grey_pixel]);
    }
    for outline in boxes {
        draw_box(&mut rgb, width, height, *outline);
    }
    rgb
}
fn draw_box(rgb: &mut [u8], width: usize, height: usize, outline: [usize; 4]) {
    let [x1, y1, x2, y2] = outline;
    if x2 <= x1 || y2 <= y1 {
        return;
    }
    let (col_low, col_high) = (x1.min(width), x2.min(width));
    let (row_low, row_high) = (y1.min(height), y2.min(height));
    for y in [y1, y2 - 1] {
        if y < height {
            for x in col_low..col_high {
                paint_one(rgb, width, x, y);
            }
        }
    }
    for x in [x1, x2 - 1] {
        if x < width {
            for y in row_low..row_high {
                paint_one(rgb, width, x, y);
            }
        }
    }
}
fn paint_one(rgb: &mut [u8], width: usize, x: usize, y: usize) {
    let start = (y * width + x) * 3;
    if let Some(pixel) = rgb.get_mut(start..start + 3) {
        pixel.copy_from_slice(&RED);
    }
}
