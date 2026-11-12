use tablet::U256;

pub fn call(url: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let response = ureq::post(url)
        .timeout(std::time::Duration::from_secs(20))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|err| format!("节点请求失败：{err}"))?;
    let value: serde_json::Value = response
        .into_json()
        .map_err(|err| format!("节点返回无法解析：{err}"))?;
    if let Some(err) = value.get("error") {
        return Err(format!("节点拒绝：{err}"));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "节点没有返回结果".into())
}

pub fn chain_id(url: &str) -> Result<u64, String> {
    let result = call(url, "eth_chainId", serde_json::json!([]))?;
    parse_u64_hex(result.as_str().ok_or("链编号不是字符串")?)
}

pub fn nonce(url: &str, from: &str) -> Result<u64, String> {
    let result = call(
        url,
        "eth_getTransactionCount",
        serde_json::json!([from, "pending"]),
    )?;
    parse_u64_hex(result.as_str().ok_or("nonce 不是字符串")?)
}

pub fn base_fee(url: &str) -> Result<Option<U256>, String> {
    let result = call(
        url,
        "eth_getBlockByNumber",
        serde_json::json!(["latest", false]),
    )?;
    let Some(value) = result.get("baseFeePerGas") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(parse_u256_hex(
        value.as_str().ok_or("baseFee 不是字符串")?,
    )?))
}

pub fn gas_price(url: &str) -> Result<U256, String> {
    let result = call(url, "eth_gasPrice", serde_json::json!([]))?;
    parse_u256_hex(result.as_str().ok_or("gasPrice 不是字符串")?)
}

pub fn priority_fee(url: &str) -> Option<U256> {
    let result = call(url, "eth_maxPriorityFeePerGas", serde_json::json!([])).ok()?;
    parse_u256_hex(result.as_str()?).ok()
}

pub fn send_raw(url: &str, raw: &[u8]) -> Result<String, String> {
    let result = call(
        url,
        "eth_sendRawTransaction",
        serde_json::json!([hex_encode(raw)]),
    )?;
    result
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "广播结果不是交易哈希".into())
}

pub fn parse_u64_hex(input: &str) -> Result<u64, String> {
    parse_u256_hex(input)?
        .to_u64()
        .ok_or_else(|| "数值超出范围".into())
}

pub fn parse_u256_hex(input: &str) -> Result<U256, String> {
    let digits = input
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    if digits.is_empty() {
        return Ok(U256::zero());
    }
    if digits.len() > 64 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("十六进制数值不合法".into());
    }
    let padded = format!("{digits:0>64}");
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&padded[i * 2..i * 2 + 2], 16)
            .map_err(|_| "十六进制数值不合法".to_string())?;
    }
    Ok(U256::from_be_bytes(bytes))
}

pub fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 + data.len() * 2);
    out.push_str("0x");
    for byte in data {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn parse_address(input: &str) -> Result<[u8; 20], String> {
    let digits = input
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    if digits.len() != 40 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("地址必须是 20 字节十六进制".into());
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16)
            .map_err(|_| "地址必须是 20 字节十六进制".to_string())?;
    }
    if out.iter().all(|b| *b == 0) {
        return Err("地址为空".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_quantities() {
        assert_eq!(parse_u64_hex("0x10").unwrap(), 16);
        assert_eq!(parse_u256_hex("0x0").unwrap(), U256::zero());
        let addr = parse_address("0x0000000000000000000000000000000000000001").unwrap();
        assert_eq!(addr[19], 1);
        assert!(parse_address("0x0").is_err());
        assert!(parse_u256_hex("0xgg").is_err());
    }
}
