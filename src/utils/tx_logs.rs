
use std::sync::Arc;
use log::debug;

use ethers::{providers::{Middleware, Provider, Ws}, types::{BlockId, BlockNumber, CallConfig, CallFrame, CallLogFrame, GethDebugBuiltInTracerConfig, GethDebugBuiltInTracerType, GethDebugTracerConfig, GethDebugTracerType, GethDebugTracingCallOptions, GethTrace, GethTraceFrame, Transaction},};

  pub async fn get_logs(
        wss: &Arc<Provider<Ws>>,
        tx: &Transaction,
        block_num: BlockNumber,
    ) -> Option<Vec<CallLogFrame>> {
    // add statediff trace to each transaction

    let mut trace_ops = GethDebugTracingCallOptions::default();
    let mut call_config = CallConfig::default();
    call_config.with_log = Some(true);

    trace_ops.tracing_options.tracer = Some(GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::CallTracer));
    trace_ops.tracing_options.tracer_config = Some(
        GethDebugTracerConfig::BuiltInTracer(
            GethDebugBuiltInTracerConfig::CallTracer(
                call_config
            )
        )
    );
    let block_num = Some(BlockId::Number(block_num));

    let call_frame = match wss.debug_trace_call(tx, block_num, trace_ops).await {
        Ok(d) => {
            match d {
                GethTrace::Known(d) => {
                    match d {
                        GethTraceFrame::CallTracer(d) => d,
                        _ => return None
                    }
                }
                _ => return None
            }
        },
        Err(_) => {
            debug!("Failed to get trace for transaction: {}", tx.hash);
            return None
        }
    }; 

    let mut logs = Vec::new();
    extract_logs(&call_frame, &mut logs);
    
    
    Some(logs)
}

fn extract_logs(call_frame: &CallFrame, logs: &mut Vec<CallLogFrame>) {
    if let Some(ref logs_vec) = call_frame.logs {
        logs.extend(logs_vec.iter().cloned());
    }

    if let Some(ref calls_vec) = call_frame.calls {
        for call in calls_vec {
            extract_logs(call, logs);
        }
    }
}
