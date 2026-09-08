const MIDDLEWARE_FILE_LIMIT: u64 = 16 * 1024 * 1024;

pub(super) fn read_middleware_bytes<R: std::io::Read>(reader: R) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut bounded = std::io::Read::take(reader, MIDDLEWARE_FILE_LIMIT + 1);
    std::io::Read::read_to_end(&mut bounded, &mut bytes)
        .map_err(|e| format!("read middleware request: {e}"))?;
    if bytes.len() as u64 > MIDDLEWARE_FILE_LIMIT {
        return Err("middleware request exceeds 16 MiB admission bound".into());
    }
    Ok(bytes)
}

pub(super) fn read_middleware_request<T: serde::de::DeserializeOwned>(
    file: &str,
) -> Result<T, String> {
    let bytes = if file == "-" {
        read_middleware_bytes(std::io::stdin().lock()).map_err(|e| {
            e.replacen(
                "read middleware request",
                "read middleware request from stdin",
                1,
            )
        })?
    } else {
        let handle =
            std::fs::File::open(file).map_err(|e| format!("read middleware request: {e}"))?;
        read_middleware_bytes(handle)?
    };
    serde_json::from_slice(&bytes).map_err(|e| format!("parse middleware request: {e}"))
}
