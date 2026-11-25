//! MJPEG decoding with `zune-jpeg` (docs/spec/v1.md §3.2).
//!
//! Used on Windows, where xReveal takes the card's compressed frames and decodes them itself:
//! macOS always decodes MJPEG before the app sees it. Output is interleaved Y, Cb, Cr per pixel,
//! uploaded as-is and turned into colour by the video shader (layout `Ycbcr444`).

use std::io::Cursor;

use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

/// Decodes JPEG frames into one reused buffer.
#[derive(Default)]
pub struct MjpegDecoder {
    out: Vec<u8>,
}

/// A decoded frame: `width` × `height` pixels of (Y, Cb, Cr) bytes, full range as JPEG defines.
pub struct Decoded<'a> {
    pub width: u32,
    pub height: u32,
    pub ycbcr: &'a [u8],
}

impl MjpegDecoder {
    pub fn decode(&mut self, jpeg: &[u8]) -> Result<Decoded<'_>, String> {
        // MJPEG frames often leave out their Huffman tables; zune-jpeg fills in the standard ones.
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::YCbCr);
        let mut decoder = JpegDecoder::new_with_options(Cursor::new(jpeg), options);
        decoder.decode_headers().map_err(|e| format!("MJPEG headers: {e:?}"))?;
        let info = decoder.info().ok_or("MJPEG frame has no size")?;
        let size = decoder.output_buffer_size().ok_or("MJPEG frame too large")?;
        self.out.resize(size, 0);
        decoder.decode_into(&mut self.out).map_err(|e| format!("MJPEG decode: {e:?}"))?;
        Ok(Decoded { width: info.width as u32, height: info.height as u32, ycbcr: &self.out })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of the colour bars (scripts/make-test-clips.sh), full range, BT.709.
    const BARS: &[u8] = include_bytes!("../../tests/clips/bars-1280x720-full.jpg");
    /// White, red, green, blue, black in full-range BT.709 (Y, Cb, Cr).
    const FULL: [(u8, u8, u8); 5] = [(255, 128, 128), (54, 98, 255), (182, 30, 12), (18, 255, 116), (0, 128, 128)];

    #[test]
    fn decodes_the_bars_to_ycbcr() {
        let mut decoder = MjpegDecoder::default();
        let frame = decoder.decode(BARS).expect("decodes");
        assert_eq!((frame.width, frame.height), (1280, 720));
        assert_eq!(frame.ycbcr.len(), 1280 * 720 * 3);
        let row = 360;
        for (bar, want) in FULL.iter().enumerate() {
            let x = 1280 * (2 * bar + 1) / 10;
            let p = (row * 1280 + x) * 3;
            let got = (frame.ycbcr[p], frame.ycbcr[p + 1], frame.ycbcr[p + 2]);
            let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 6;
            assert!(close(got.0, want.0) && close(got.1, want.1) && close(got.2, want.2), "bar {bar}: {got:?} vs {want:?}");
        }
    }

    #[test]
    fn reuses_its_buffer_and_rejects_garbage() {
        let mut decoder = MjpegDecoder::default();
        decoder.decode(BARS).expect("first");
        decoder.decode(BARS).expect("second");
        assert!(decoder.decode(b"not a jpeg").is_err());
    }
}
