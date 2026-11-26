//! Cursor-compositing probe: fills a 256×256 BGRA test frame with a
//! checkerboard, composites the current system cursor at its live position
//! and writes `cursor_probe.jpg` for visual inspection.
//!
//! Move the mouse over the top-left 256×256 area of the primary monitor
//! before running (e.g. `SetCursorPos 64 64`), then:
//!   cargo run --example cursor_probe   (from src-tauri)

use HiScreen_lib::cursor::{draw_cursor, monitor_origin};

fn main() {
    let (w, h) = (256usize, 256usize);
    // BGRA checkerboard (bluish) so both cursor fill and its transparent
    // parts are easy to judge.
    let mut frame = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let v = if (x / 16 + y / 16) % 2 == 0 { 110 } else { 170 };
            frame[i] = 210; // B
            frame[i + 1] = v; // G
            frame[i + 2] = v; // R
            frame[i + 3] = 255;
        }
    }

    let origin = monitor_origin(0);
    println!("monitor_origin(0) = {origin:?}");
    match origin {
        Some(o) => draw_cursor(&mut frame, w as i32, h as i32, o),
        None => eprintln!("!! monitor_origin unavailable - cursor would be skipped"),
    }

    let mut jpg = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut jpg, 90);
    enc.encode(&frame, w as u16, h as u16, jpeg_encoder::ColorType::Bgra)
        .expect("encode failed");
    std::fs::write("cursor_probe.jpg", &jpg).expect("write failed");
    println!("wrote cursor_probe.jpg");
}
