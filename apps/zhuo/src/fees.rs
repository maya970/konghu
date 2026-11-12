use std::cmp::Ordering;
use tablet::U256;

pub struct FeeProposal {
    pub priority: U256,
    pub max_fee: U256,
    pub fee_cap: U256,
}

pub fn propose_fees(
    base: Option<U256>,
    gas_price: Option<U256>,
    priority_hint: Option<U256>,
    gas_limit: u64,
) -> Result<FeeProposal, String> {
    let one_gwei = U256::from_u64(1_000_000_000);
    let mut priority = priority_hint.unwrap_or(one_gwei);
    let mut max_fee = if let Some(base) = base {
        base.checked_mul_u64(2)
            .and_then(|doubled| doubled.checked_add(priority))
            .ok_or("费用溢出")?
    } else if let Some(gas_price) = gas_price {
        let doubled = gas_price.checked_mul_u64(2).ok_or("费用溢出")?;
        if priority.cmp(doubled) == Ordering::Greater {
            priority = gas_price;
        }
        doubled
    } else {
        return Err("节点没有给出费用".into());
    };
    if priority.cmp(max_fee) == Ordering::Greater {
        priority = max_fee;
    }
    if max_fee.is_zero() {
        max_fee = U256::from_u64(1);
        priority = U256::zero();
    }
    let fee_cap = max_fee.checked_mul_u64(gas_limit).ok_or("费用上限溢出")?;
    Ok(FeeProposal {
        priority,
        max_fee,
        fee_cap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fees_follow_the_node_and_cover_gas() {
        let base = U256::from_u64(1_000);
        let proposal = propose_fees(Some(base), None, Some(U256::from_u64(5)), 21_000).unwrap();
        assert_eq!(proposal.max_fee, U256::from_u64(2_005));
        assert_eq!(proposal.fee_cap, U256::from_u64(2_005 * 21_000));

        let zero = propose_fees(Some(U256::zero()), None, Some(U256::zero()), 21_000).unwrap();
        assert_eq!(zero.max_fee, U256::from_u64(1));
        assert!(propose_fees(None, None, None, 21_000).is_err());
    }
}
