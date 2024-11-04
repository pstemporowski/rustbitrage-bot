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
use log::{debug, error};
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
    let geth_trace = match provider
        .debug_trace_call(
            tx.into(),
            BlockId::Number(BlockNumberOrTag::Latest),
            GethDebugTracingCallOptions {
                tracing_options,
                state_overrides: None,
                block_overrides: None,
            },
        )
        .await
    {
        Ok(trace) => trace,
        Err(e) => {
            debug!("Failed to get geth trace: {}", e);
            return Err(e.into());
        }
    };

    // Convert trace result into call frame
    let call_frame = match geth_trace.try_into_call_frame() {
        Ok(frame) => frame,
        Err(e) => {
            error!("Failed to convert geth trace to call frame: {}", e);
            return Err(e.into());
        }
    };

    Ok(call_frame)
}
