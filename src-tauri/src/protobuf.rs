/// Protobuf varint encoding.
pub fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    while value >= 0x80 {
        buf.push((value & 0x7F | 0x80) as u8);
        value >>= 7;
    }
    buf.push(value as u8);
    buf
}

/// Encode a length-delimited field (wire_type = 2).
pub fn encode_len_delim_field(field_num: u32, data: &[u8]) -> Vec<u8> {
    let tag = (field_num << 3) | 2;
    let mut f = encode_varint(tag as u64);
    f.extend(encode_varint(data.len() as u64));
    f.extend_from_slice(data);
    f
}

/// Encode a string field (wire_type = 2).
pub fn encode_string_field(field_num: u32, value: &str) -> Vec<u8> {
    encode_len_delim_field(field_num, value.as_bytes())
}

/// Read a protobuf varint from `data` starting at `offset`.
pub fn read_varint(data: &[u8], offset: usize) -> Result<(u64, usize), String> {
    let mut result = 0u64;
    let mut shift = 0;
    let mut pos = offset;

    loop {
        if pos >= data.len() {
            return Err("varint: data truncated".to_string());
        }
        let byte = data[pos];
        result |= ((byte & 0x7F) as u64) << shift;
        pos += 1;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }

    Ok((result, pos))
}

/// Skip a protobuf field based on its wire type.
pub fn skip_field(data: &[u8], offset: usize, wire_type: u8) -> Result<usize, String> {
    match wire_type {
        0 => {
            // Varint
            let (_, new_offset) = read_varint(data, offset)?;
            Ok(new_offset)
        }
        1 => {
            // 64-bit fixed
            Ok(offset + 8)
        }
        2 => {
            // Length-delimited
            let (length, content_offset) = read_varint(data, offset)?;
            Ok(content_offset + length as usize)
        }
        5 => {
            // 32-bit fixed
            Ok(offset + 4)
        }
        _ => Err(format!("unknown wire_type: {}", wire_type)),
    }
}

/// Create an OAuthTokenInfo protobuf message.
///
/// Fields:
///   1 = access_token (string)
///   2 = token_type   (string, always "Bearer")
///   3 = refresh_token (string)
///   4 = expiry (nested Timestamp: field 1 = seconds as varint)
pub fn create_oauth_info(access_token: &str, refresh_token: &str, expiry: i64) -> Vec<u8> {
    let field1 = encode_string_field(1, access_token);
    let field2 = encode_string_field(2, "Bearer");
    let field3 = encode_string_field(3, refresh_token);

    // Nested Timestamp message: field 1 = seconds (varint)
    let timestamp_tag = (1 << 3) | 0; // field 1, varint
    let mut timestamp_msg = encode_varint(timestamp_tag);
    timestamp_msg.extend(encode_varint(expiry as u64));
    let field4 = encode_len_delim_field(4, &timestamp_msg);

    [field1, field2, field3, field4].concat()
}
