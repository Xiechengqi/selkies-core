use criterion::{criterion_group, criterion_main, Criterion};
use selkies_core::encode::{Encoder, EncoderConfig};
use selkies_core::capture::Frame;
use std::time::Instant;

fn bench_jpeg_stripe_encoding(c: &mut Criterion) {
    let width = 1920;
    let height = 1080;
    let data = vec![128u8; width * height * 3];
    let frame = Frame {
        width: width as u32,
        height: height as u32,
        data,
        timestamp: Instant::now(),
        sequence: 0,
        is_dirty: true,
    };

    let config = EncoderConfig {
        quality: 80,
        stripe_height: 64,
        subsample: 1,
    };
    let mut encoder = Encoder::new(config).expect("encoder init");

    c.bench_function("encode_1080p_frame", |b| {
        b.iter(|| {
            let _ = encoder.encode_frame(&frame).expect("encode frame");
        })
    });
}

criterion_group!(benches, bench_jpeg_stripe_encoding);
criterion_main!(benches);
