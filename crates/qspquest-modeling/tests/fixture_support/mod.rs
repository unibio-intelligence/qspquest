use std::io::Read;

pub fn text(bytes: &[u8]) -> String {
    let mut value = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut value)
        .unwrap();
    value
}
