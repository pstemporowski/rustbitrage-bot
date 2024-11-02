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
use log::{debug, info};
use std::sync::Arc;

/// Retrieves a call frame for a given transaction by performing debug tracing
///
/// # Arguments
/// * `tx` - The transaction to trace
/// * `provider` - The RPC provider to use for tracing
///
/// # Returns
/// * `eyre::Result<CallFrame>` - The resulting call frame or error
pub async fn get_call_frame(
    tx: Transaction,
    provider: Arc<RootProvider<PubSubFrontend>>,
) -> eyre::Result<CallFrame> {
    info!("Getting call frame for transaction: {:?}", tx.hash);

    // Configure tracing options - disable memory, return data, storage and stack for performance
    let trace_options = GethDefaultTracingOptions {
        enable_memory: Some(false),
        enable_return_data: Some(false),
        disable_storage: Some(true),
        disable_stack: Some(true),
        ..Default::default()
    };

    // Set up debug tracing with call tracer and timeout
    let tracing_options = GethDebugTracingOptions {
        tracer: Some(GethDebugTracerType::BuiltInTracer(
            GethDebugBuiltInTracerType::CallTracer,
        )),
        timeout: Some("10s".to_string()),
        config: trace_options,
        tracer_config: GethDebugTracerConfig::default(),
    };

    // Execute debug trace call using latest block since transaction is pending
    let geth_trace = provider
        .debug_trace_call(
            tx.into(),
            BlockId::Number(BlockNumberOrTag::Pending),
            GethDebugTracingCallOptions {
                tracing_options,
                state_overrides: None,
                block_overrides: None,
            },
        )
        .await?;

    debug!("Successfully retrieved geth trace");

    // Convert trace result into call frame
    let call_frame = geth_trace.try_into_call_frame()?;
    info!("Successfully converted trace to call frame");

    Ok(call_frame)
}
