#![allow(dead_code)]

mod cost_types;
mod measure;
mod modelfit;
mod util;

use cost_types::*;
pub use measure::*;
pub use modelfit::*;

use soroban_env_host::{
    cost_runner::{CostRunner, WasmInsnType},
    xdr::ContractCostType,
};
use std::collections::BTreeMap;

pub(crate) trait Benchmark {
    fn bench<HCM: HostCostMeasurement>() -> std::io::Result<(FPCostModel, FPCostModel)>;
}

fn get_explicit_bench_names() -> Option<Vec<String>> {
    let bare_args: Vec<_> = std::env::args().filter(|x| !x.starts_with("-")).collect();
    match bare_args.len() {
        0 | 1 => None,
        _ => Some(bare_args[1..].into()),
    }
}

fn should_run<HCM: HostCostMeasurement>() -> bool {
    if let Some(bench_names) = get_explicit_bench_names() {
        let name = format!("{:?}", <HCM::Runner as CostRunner>::COST_TYPE)
            .split("::")
            .last()
            .unwrap()
            .to_string();
        bench_names.iter().any(|arg| *arg == name)
    } else {
        true
    }
}

fn should_run_wasm_insn(ty: WasmInsnType) -> bool {
    if let Some(bench_names) = get_explicit_bench_names() {
        let name = format!("{:?}", ty).split("::").last().unwrap().to_string();
        bench_names.iter().any(|arg| *arg == name)
    } else {
        true
    }
}

fn call_bench<B: Benchmark, HCM: HostCostMeasurement>(
    params: &mut BTreeMap<ContractCostType, (FPCostModel, FPCostModel)>,
) -> std::io::Result<()> {
    if should_run::<HCM>() {
        params.insert(<HCM::Runner as CostRunner>::COST_TYPE, B::bench::<HCM>()?);
    }
    Ok(())
}

pub(crate) fn for_each_host_cost_measurement<B: Benchmark>(
) -> std::io::Result<BTreeMap<ContractCostType, (FPCostModel, FPCostModel)>> {
    let mut params: BTreeMap<ContractCostType, (FPCostModel, FPCostModel)> = BTreeMap::new();

    call_bench::<B, ComputeEd25519PubKeyMeasure>(&mut params)?;
    call_bench::<B, ComputeSha256HashMeasure>(&mut params)?;
    call_bench::<B, VerifyEd25519SigMeasure>(&mut params)?;
    call_bench::<B, VmInstantiationMeasure>(&mut params)?;
    call_bench::<B, VmMemReadMeasure>(&mut params)?;
    call_bench::<B, VmMemWriteMeasure>(&mut params)?;
    call_bench::<B, WasmInsnExecMeasure>(&mut params)?;
    call_bench::<B, WasmMemAllocMeasure>(&mut params)?;
    call_bench::<B, VisitObjectMeasure>(&mut params)?;
    call_bench::<B, GuardFrameMeasure>(&mut params)?;
    call_bench::<B, ValXdrConvMeasure>(&mut params)?;
    call_bench::<B, ValSerMeasure>(&mut params)?;
    call_bench::<B, ValDeserMeasure>(&mut params)?;
    call_bench::<B, MapEntryMeasure>(&mut params)?;
    call_bench::<B, VecEntryMeasure>(&mut params)?;
    call_bench::<B, HostMemCmpMeasure>(&mut params)?;
    call_bench::<B, InvokeVmFunctionMeasure>(&mut params)?;
    call_bench::<B, InvokeHostFunctionMeasure>(&mut params)?;
    call_bench::<B, ChargeBudgetMeasure>(&mut params)?;
    call_bench::<B, HostMemAllocMeasure>(&mut params)?;
    call_bench::<B, HostMemCpyMeasure>(&mut params)?;

    if get_explicit_bench_names().is_none() {
        for cost in ContractCostType::variants() {
            if !params.contains_key(&cost) {
                eprintln!("warning: missing cost measurement for {:?}", cost);
            }
        }
    }
    Ok(params)
}

macro_rules! run_wasm_insn_measurement {
    ( $($HCM: ident),* ) => {
        pub(crate) fn for_each_wasm_insn_measurement<B: Benchmark>() -> std::io::Result<BTreeMap<WasmInsnType, (FPCostModel, FPCostModel)>> {
            let mut params: BTreeMap<WasmInsnType, (FPCostModel, FPCostModel)> = BTreeMap::new();
            $(
                let ty = <$HCM as HostCostMeasurement>::Runner::INSN_TYPE;
                if should_run_wasm_insn(ty) {
                    eprintln!(
                        "\nMeasuring costs for WasmInsnType::{:?}\n", ty);
                    params.insert(ty, B::bench::<$HCM>()?);
                }
            )*

            if get_explicit_bench_names().is_none() {
                for insn in WasmInsnType::variants() {
                    if !params.contains_key(insn) {
                        eprintln!("warning: missing cost measurement for {:?}", insn);
                    }
                }
            }
            Ok(params)
        }
    };
}
run_wasm_insn_measurement!(
    WasmSelectMeasure,
    WasmBrMeasure,
    WasmBrTableMeasure,
    WasmConstMeasure,
    WasmDropMeasure,
    WasmLocalGetMeasure,
    WasmLocalSetMeasure,
    WasmLocalTeeMeasure,
    WasmCallMeasure,
    WasmCallIndirectMeasure,
    WasmGlobalGetMeasure,
    WasmGlobalSetMeasure,
    WasmI64StoreMeasure,
    WasmI64Store8Measure,
    WasmI64Store16Measure,
    WasmI64Store32Measure,
    WasmI64LoadMeasure,
    WasmI64Load8Measure,
    WasmI64Load16Measure,
    WasmI64Load32Measure,
    WasmMemorySizeMeasure,
    WasmMemoryGrowMeasure,
    WasmI64ClzMeasure,
    WasmI64CtzMeasure,
    WasmI64PopcntMeasure,
    WasmI64EqzMeasure,
    WasmI64EqMeasure,
    WasmI64NeMeasure,
    WasmI64LtSMeasure,
    WasmI64GtSMeasure,
    WasmI64LeSMeasure,
    WasmI64GeSMeasure,
    WasmI64AddMeasure,
    WasmI64SubMeasure,
    WasmI64MulMeasure,
    WasmI64DivSMeasure,
    WasmI64RemSMeasure,
    WasmI64AndMeasure,
    WasmI64OrMeasure,
    WasmI64XorMeasure,
    WasmI64ShlMeasure,
    WasmI64ShrSMeasure,
    WasmI64RotlMeasure,
    WasmI64RotrMeasure
);
