use std::time::Instant;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let data: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let avg = data.iter().map(|d| d.len()).sum::<usize>() / data.len();
    println!("{} frames, avg {} KB", data.len(), avg / 1024);
    for (name, cs) in [("zune RGB", ColorSpace::RGB), ("zune YCbCr(interleaved)", ColorSpace::YCbCr), ("zune RGBA", ColorSpace::RGBA)] {
        let opts = DecoderOptions::default().jpeg_set_out_colorspace(cs);
        let mut out = vec![0u8; 1920 * 1080 * 4];
        let t = Instant::now();
        for _ in 0..3 { for d in &data {
            let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(d.as_slice()), opts);
            dec.decode_headers().unwrap();
            let n = dec.output_buffer_size().unwrap();
            dec.decode_into(&mut out[..n]).unwrap();
        }}
        println!("{name}: {:.2} ms/frame", t.elapsed().as_secs_f64() * 1000.0 / (3 * data.len()) as f64);
    }
    let t = Instant::now();
    for _ in 0..3 { for d in &data {
        let mut dec = jpeg_decoder::Decoder::new(d.as_slice());
        let _ = dec.decode().unwrap();
    }}
    println!("jpeg-decoder RGB (no rayon): {:.2} ms/frame", t.elapsed().as_secs_f64() * 1000.0 / (3 * data.len()) as f64);
}
