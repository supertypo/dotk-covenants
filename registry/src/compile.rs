use anyhow::{Result, anyhow, ensure};
use silverscript_abi::SilAbiArtifact;
use silverscript_lang::ast::Expr;
use silverscript_lang::compiler::{CompileOptions, compile_contract, sil_abi_artifact_from_compiled};

use crate::api::Params;
use crate::state::{DEED_STATE_LEN, GAP_STATE_LEN, KEY_MAX, KEY_MIN, ZERO32};

pub const GAP_SOURCE: &str = include_str!("../sil/dotk_gap.sil");
pub const DEED_SOURCE: &str = include_str!("../sil/dotk_deed.sil");

/// The deed's placeholder state. Only the bytes around the state region matter.
const PENDING: u8 = 0x01;
const OWNER_PUBKEY: u8 = 0x00;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub bytecode: Vec<u8>,
    pub state_start: usize,
    pub state_len: usize,
}

impl Template {
    pub fn prefix(&self) -> &[u8] {
        &self.bytecode[..self.state_start]
    }

    pub fn suffix(&self) -> &[u8] {
        &self.bytecode[self.state_start + self.state_len..]
    }

    pub fn template_hash(&self) -> [u8; 32] {
        silverscript_abi::template_hash(self.prefix(), self.suffix())
    }

    pub fn materialize(&self, state: &[u8]) -> Result<Vec<u8>> {
        ensure!(state.len() == self.state_len, "state must be {} bytes, got {}", self.state_len, state.len());
        let mut script = self.bytecode.clone();
        script[self.state_start..self.state_start + self.state_len].copy_from_slice(state);
        Ok(script)
    }

    pub fn p2sh_spk(&self, state: &[u8]) -> Result<Vec<u8>> {
        let redeem = self.materialize(state)?;
        Ok(kaspa_txscript::pay_to_script_hash_script(&redeem).script().to_vec())
    }
}

pub struct Templates {
    pub gap: Template,
    pub deed: Template,
    /// The artifacts a deployment publishes for these templates.
    pub gap_abi: SilAbiArtifact,
    pub deed_abi: SilAbiArtifact,
}

fn bytes32(v: &[u8; 32]) -> Expr<'static> {
    Expr::bytes(v.to_vec())
}

/// The parameters compile to signed integers, so no deployment used a value above `i64::MAX`.
fn int(label: &str, v: u64) -> Result<Expr<'static>> {
    let v = i64::try_from(v).map_err(|_| anyhow!("{label} is {v}, which does not fit a script integer"))?;
    Ok(Expr::int(v))
}

/// The gap embeds the deed's template hash and lengths, so the deed compiles first.
pub fn compile(params: &Params) -> Result<Templates> {
    let mut devfund = vec![0u8, 0u8];
    devfund.extend(crate::hex::decode(&params.devfund_spk)?);
    let deed_ctor = vec![
        Expr::dynamic_bytes(devfund),
        int("fee_1ch", params.fee_1ch)?,
        int("fee_2ch", params.fee_2ch)?,
        int("fee_3ch", params.fee_3ch)?,
        int("fee_4ch", params.fee_4ch)?,
        int("fee_5plus", params.fee_5plus)?,
        int("bond", params.bond)?,
        int("deposit", params.deposit)?,
        int("t_evict", params.t_evict)?,
        Expr::byte(PENDING),
        bytes32(&ZERO32),
        Expr::byte(OWNER_PUBKEY),
        bytes32(&ZERO32),
        bytes32(&ZERO32),
    ];
    let deed_contract =
        compile_contract(DEED_SOURCE, &deed_ctor, CompileOptions::default()).map_err(|e| anyhow!("compiling DotkDeed: {e}"))?;
    let deed_abi = sil_abi_artifact_from_compiled(&deed_contract, &deed_ctor).map_err(|e| anyhow!("DotkDeed artifact: {e}"))?;
    let deed = Template {
        bytecode: deed_contract.bytecode.clone(),
        state_start: deed_contract.state_layout.start,
        state_len: deed_contract.state_layout.len,
    };
    ensure!(deed.state_len == DEED_STATE_LEN, "DotkDeed state is {} bytes, expected {DEED_STATE_LEN}", deed.state_len);

    let gap_ctor = vec![
        int("bond", params.bond)?,
        int("deposit", params.deposit)?,
        int("gap_value", params.gap_value)?,
        bytes32(&deed.template_hash()),
        Expr::int(deed.prefix().len() as i64),
        Expr::int(deed.suffix().len() as i64),
        bytes32(&KEY_MIN),
        bytes32(&KEY_MAX),
    ];
    let gap_contract =
        compile_contract(GAP_SOURCE, &gap_ctor, CompileOptions::default()).map_err(|e| anyhow!("compiling DotkGap: {e}"))?;
    let gap_abi = sil_abi_artifact_from_compiled(&gap_contract, &gap_ctor).map_err(|e| anyhow!("DotkGap artifact: {e}"))?;
    let gap = Template {
        bytecode: gap_contract.bytecode.clone(),
        state_start: gap_contract.state_layout.start,
        state_len: gap_contract.state_layout.len,
    };
    ensure!(gap.state_len == GAP_STATE_LEN, "DotkGap state is {} bytes, expected {GAP_STATE_LEN}", gap.state_len);

    Ok(Templates { gap, deed, gap_abi, deed_abi })
}
