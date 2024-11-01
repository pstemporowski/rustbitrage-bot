use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    providers::{ext::DebugApi, RootProvider},
    pubsub::PubSubFrontend,
    rpc::types::{
        trace::geth::{
            CallFrame, GethDebugBuiltInTracerType, GethDebugTracerConfig, GethDebugTracerType,
            GethDebugTracingCallOptions, GethDebugTracingOptions, GethDefaultTracingOptions,
        },
        Transaction,
    },
};
use std::sync::Arc;

pub async fn get_call_frame(
    tx: Transaction,
    provider: Arc<RootProvider<PubSubFrontend>>,
) -> eyre::Result<CallFrame> {
    let trace_options = GethDefaultTracingOptions {
        enable_memory: Some(false),
        enable_return_data: Some(false),
        disable_storage: Some(true),
        disable_stack: Some(true),
        ..Default::default()
    };

    let tracing_options = GethDebugTracingOptions {
        tracer: Some(GethDebugTracerType::BuiltInTracer(
            GethDebugBuiltInTracerType::CallTracer,
        )),
        timeout: Some("10s".to_string()),
        config: trace_options,
        tracer_config: GethDebugTracerConfig::default(),
    };

    let block_number = tx.block_number.unwrap_or(0);

    let geth_trace = provider
        .debug_trace_call(
            tx.into(),
            BlockId::Number(BlockNumberOrTag::Number(block_number)),
            GethDebugTracingCallOptions {
                tracing_options,
                state_overrides: None,
                block_overrides: None,
            },
        )
        .await?;

    let call_frame = geth_trace.try_into_call_frame()?;

    Ok(call_frame)
}
