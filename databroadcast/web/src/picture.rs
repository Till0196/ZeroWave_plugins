// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 放送の画像をデコードする。ARIB の PNG は色を文書の色のテーブル（CLUT）で決める。JPEG はそのまま。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    pub fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            if let Ok(mut writer) = encoder.write_header() {
                let _ = writer.write_image_data(&self.rgba);
            }
        }
        out
    }
}

/// 画像の幅・高さの上限と、画素の数の上限。放送の画像はずっと小さい。
pub const SIDE_MOST: u32 = 8192;
pub const PIXELS_MOST: u64 = 3840 * 2160 * 2;

pub fn fits(width: u32, height: u32) -> bool {
    width <= SIDE_MOST && height <= SIDE_MOST && u64::from(width) * u64::from(height) <= PIXELS_MOST
}

/// PNG か JPEG を RGBA に。番号の PNG は PLTE/tRNS を見ず、文書の色のテーブルで色にする
/// （STD-B24 第二分冊 (2/2) 付属 4）。
pub fn decode(
    content_type: &str,
    bytes: &[u8],
    name: &str,
    clut: &bml::media::clut::Clut,
) -> Option<Picture> {
    // MNG は `name#n` の n 番目のフレーム（無ければ最初）。
    if bml::media::mng::is_mng(bytes) {
        let index: usize = name
            .rsplit_once('#')
            .and_then(|(_, n)| n.parse().ok())
            .unwrap_or(0);
        let png = bml::media::mng::frame(bytes, index)?;
        return decode_png(&png, clut)
            .map_err(|error| {
                zwsdk::plug::warn(&format!(
                    "データ放送  {name}: MNG のフレームが読めない: {error}"
                ))
            })
            .ok();
    }
    let kind = content_type.to_ascii_lowercase();
    if kind.contains("png") || bytes.starts_with(b"\x89PNG") {
        return match decode_png(bytes, clut) {
            Ok(picture) => Some(picture),
            Err(error) => {
                zwsdk::plug::warn(&format!("データ放送  {name}: PNG が読めない: {error}"));
                None
            }
        };
    }
    if kind.contains("jpeg") || kind.contains("jpg") || bytes.starts_with(&[0xff, 0xd8]) {
        let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
        decoder.set_options(
            zune_core::options::DecoderOptions::default()
                .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA)
                .set_max_width(SIDE_MOST as usize)
                .set_max_height(SIDE_MOST as usize),
        );
        if decoder.decode_headers().is_ok()
            && let Some((width, height)) = decoder.dimensions()
            && !fits(width as u32, height as u32)
        {
            zwsdk::plug::warn(&format!(
                "データ放送  {name}: JPEG が大きすぎる（{width}×{height}）"
            ));
            return None;
        }
        let pixels = match decoder.decode() {
            Ok(pixels) => pixels,
            Err(error) => {
                zwsdk::plug::warn(&format!("データ放送  {name}: JPEG が読めない: {error:?}"));
                return None;
            }
        };
        let (width, height) = decoder.dimensions()?;
        return Some(Picture {
            width: width as u32,
            height: height as u32,
            rgba: pixels,
        });
    }
    if kind.contains("gif") || bytes.starts_with(b"GIF8") {
        return match decode_gif(bytes) {
            Ok(picture) => Some(picture),
            Err(error) => {
                zwsdk::plug::warn(&format!("データ放送  {name}: GIF が読めない: {error}"));
                None
            }
        };
    }
    zwsdk::plug::info(&format!(
        "データ放送  {name}: {content_type} はデコードできない"
    ));
    None
}

/// GIF の最初のフレーム。論理画面の大きさで、残りと透明色の番号は透明に。
fn decode_gif(bytes: &[u8]) -> Result<Picture, String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(std::io::Cursor::new(bytes))
        .map_err(|e| e.to_string())?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    if !fits(width, height) {
        return Err(format!("大きすぎる（{width}×{height}）"));
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    if let Some(frame) = decoder.read_next_frame().map_err(|e| e.to_string())? {
        let (left, top) = (u32::from(frame.left), u32::from(frame.top));
        let (fw, fh) = (u32::from(frame.width), u32::from(frame.height));
        for y in 0..fh {
            let Some(ty) = top.checked_add(y).filter(|ty| *ty < height) else {
                break;
            };
            for x in 0..fw {
                let Some(tx) = left.checked_add(x).filter(|tx| *tx < width) else {
                    break;
                };
                let from = (y as usize * fw as usize + x as usize) * 4;
                let to = (ty as usize * width as usize + tx as usize) * 4;
                let Some(pixel) = frame.buffer.get(from..from + 4) else {
                    break;
                };
                rgba[to..to + 4].copy_from_slice(pixel);
            }
        }
    }
    Ok(Picture {
        width,
        height,
        rgba,
    })
}

fn decode_png(bytes: &[u8], clut: &bml::media::clut::Clut) -> Result<Picture, String> {
    let indexed = bytes.get(0x19) == Some(&3);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(if indexed {
        png::Transformations::IDENTITY
    } else {
        png::Transformations::EXPAND | png::Transformations::ALPHA
    });
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = (reader.info().width, reader.info().height);
    if !fits(width, height) {
        return Err(format!("大きすぎる（{width}×{height}）"));
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("大きすぎる")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|ga| [ga[0], ga[0], ga[0], ga[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => {
            let depth = info.bit_depth as usize;
            let per_row = (info.width as usize * depth).div_ceil(8);
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for row in data.chunks(per_row) {
                for x in 0..info.width as usize {
                    let index = match depth {
                        8 => row[x],
                        4 => (row[x / 2] >> (4 - 4 * (x % 2))) & 0x0f,
                        2 => (row[x / 4] >> (6 - 2 * (x % 4))) & 0x03,
                        1 => (row[x / 8] >> (7 - (x % 8))) & 0x01,
                        _ => 0,
                    };
                    out.extend_from_slice(&clut.colour(u32::from(index)));
                }
            }
            out
        }
    };
    Ok(Picture {
        width: info.width,
        height: info.height,
        rgba,
    })
}
