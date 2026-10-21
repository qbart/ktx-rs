// Debug helper (not part of the library): transcode a KTX2 and dump the bytes.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (input, fmt, out) = (&args[1], args[2].parse::<u32>().unwrap_or(0), &args[3]);
    let data = std::fs::read(input).unwrap_or_default();
    match ktx::Texture2::create_from_memory(&data, ktx::KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
        Ok(mut t) => match t.transcode_basis(ktx::TranscodeFmt(fmt), 0) {
            Ok(()) => {
                let _ = std::fs::write(out, t.get_data().unwrap_or_default());
            }
            Err(e) => eprintln!("transcode: {e}"),
        },
        Err(e) => eprintln!("load: {e}"),
    }
}
