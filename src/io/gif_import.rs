//! Decode an animated GIF into one full-size canvas per frame.
//!
//! GIF frames are often partial sub-rectangles with a per-frame disposal method.
//! We composite each frame onto a running full-screen RGBA buffer, honoring
//! disposal, and snapshot a full canvas per frame so the importer can lay each
//! one on its own timeline frame.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};

use crate::doc::canvas::Canvas;

pub fn decode_all(path: &Path) -> Result<Vec<Canvas>> {
    let file = File::open(path).with_context(|| format!("opening {path:?}"))?;
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = opts.read_info(file).context("reading GIF header")?;

    let gw = decoder.width() as usize;
    let gh = decoder.height() as usize;

    // Running full-screen RGBA buffer that each frame composites onto.
    let mut buf = vec![0u8; gw * gh * 4];
    let mut frames: Vec<Canvas> = Vec::new();

    while let Some(frame) = decoder.read_next_frame().context("decoding GIF frame")? {
        // Snapshot before drawing — needed if this frame's disposal is "previous".
        let prev = buf.clone();

        let fx = frame.left as usize;
        let fy = frame.top as usize;
        let fw = frame.width as usize;
        let fh = frame.height as usize;

        for row in 0..fh {
            let dy = fy + row;
            if dy >= gh {
                break;
            }
            for col in 0..fw {
                let dx = fx + col;
                if dx >= gw {
                    continue;
                }
                let si = (row * fw + col) * 4;
                let a = frame.buffer[si + 3];
                // Transparent source pixel: leave whatever is underneath.
                if a == 0 {
                    continue;
                }
                let di = (dy * gw + dx) * 4;
                buf[di] = frame.buffer[si];
                buf[di + 1] = frame.buffer[si + 1];
                buf[di + 2] = frame.buffer[si + 2];
                buf[di + 3] = a;
            }
        }

        frames.push(to_canvas(&buf, gw as u32, gh as u32));

        // Prepare the buffer for the next frame per this frame's disposal.
        match frame.dispose {
            gif::DisposalMethod::Background => {
                for row in 0..fh {
                    let dy = fy + row;
                    if dy >= gh {
                        break;
                    }
                    for col in 0..fw {
                        let dx = fx + col;
                        if dx >= gw {
                            continue;
                        }
                        let di = (dy * gw + dx) * 4;
                        buf[di..di + 4].fill(0);
                    }
                }
            }
            gif::DisposalMethod::Previous => {
                buf = prev;
            }
            // Keep / Any: leave the buffer as-is.
            _ => {}
        }
    }

    Ok(frames)
}

/// Wrap a full-screen GIF RGBA buffer into a native-resolution canvas (the layer
/// transform handles placement/scale).
fn to_canvas(buf: &[u8], gw: u32, gh: u32) -> Canvas {
    let mut canvas = Canvas::new(gw, gh);
    canvas.pixels.copy_from_slice(buf);
    canvas
}
