use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Deserialize)]
pub struct Chain {
    pub chain_id: u64,
    pub symbol: String,
    pub decimals: u8,
    pub rpc: String,
}

pub fn parse_registry(text: &str) -> Result<Vec<Chain>, String> {
    if text.len() > 1_000_000 {
        return Err("登记表过大".into());
    }
    let chains: Vec<Chain> = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if chains.is_empty() || chains.len() > 5_000 {
        return Err("登记表条数不对".into());
    }
    let mut seen = HashSet::new();
    for chain in &chains {
        if chain.chain_id == 0 || !seen.insert(chain.chain_id) {
            return Err(format!("链编号重复或为空：{}", chain.chain_id));
        }
        if chain.decimals > 36 {
            return Err(format!("小数位过多：{}", chain.chain_id));
        }
        if chain.symbol.is_empty()
            || chain.symbol.len() > 16
            || !chain
                .symbol
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.')
        {
            return Err(format!("符号不合法：{}", chain.chain_id));
        }
        if !rpc_ok(&chain.rpc) {
            return Err(format!("节点地址不合法：{}", chain.chain_id));
        }
    }
    Ok(chains)
}

fn rpc_ok(rpc: &str) -> bool {
    if rpc.chars().any(|ch| ch.is_whitespace()) || rpc.len() > 200 {
        return false;
    }
    rpc.starts_with("https://") && rpc.len() > "https://".len()
        || rpc.starts_with("http://127.0.0.1")
        || rpc.starts_with("http://localhost")
}

pub fn load_registry() -> Result<Vec<Chain>, String> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("evm.json"));
            candidates.push(dir.join("registry").join("evm.json"));
        }
    }
    candidates.push(std::path::PathBuf::from("registry/evm.json"));
    for path in candidates {
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("无法读取 {}：{err}", path.display()))?;
            return parse_registry(&text).map_err(|err| format!("{}：{err}", path.display()));
        }
    }
    parse_registry(include_str!("../../../registry/evm.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_registry_is_broad_and_strict() {
        let chains = parse_registry(include_str!("../../../registry/evm.json")).unwrap();
        assert!(chains.len() >= 80);
        assert!(chains.iter().any(|chain| chain.chain_id == 1));
        assert!(parse_registry(r#"[{"chain_id":1,"symbol":"ETH","decimals":18,"rpc":"http://evil.example"}]"#).is_err());
        let mut dup = include_str!("../../../registry/evm.json").to_string();
        dup.pop();
        dup.push_str(r#",{"chain_id":1,"symbol":"ETH","decimals":18,"rpc":"https://example.test"}]"#);
        assert!(parse_registry(&dup).is_err());
    }
}
