// Shared presentation for current structured failures and retained legacy diagnostics.
const LABELS = {
  inference_timeout: ["模型执行超时", "Inference deadline reached"],
  response_timeout: ["等待响应超时，结果未确认", "Response wait expired; outcome unknown"],
  execution_interrupted: ["执行中断，可能休眠或时钟变化", "Execution interrupted; possible sleep or clock change"],
  invalid_model_output: ["模型输出未通过校验", "Model output failed validation"],
  busy: ["上游繁忙", "Upstream busy"],
  capacity: ["无可用部署", "No available deployment"],
  timeout: ["调用超时", "Request timeout"],
  protocol: ["上游响应异常", "Upstream protocol error"],
  unavailable: ["服务暂不可达", "Service unavailable"],
  other: ["其他原因", "Other causes"],
};
export function maintenanceFailureKind(reason, explicitKind) {
  if (Object.hasOwn(LABELS, explicitKind)) return explicitKind;
  const value = String(reason || "").toLowerCase();
  if (/maintenance wait interrupted/.test(value)) return "execution_interrupted";
  if (/maintenance (response wait|submission|response) timed out/.test(value)) return "response_timeout";
  if (/maintenance inference timed out|deadline_exceeded/.test(value)) return "inference_timeout";
  if (/maintenance output invalid|decode strict pcp maintenance decision/.test(value)) return "invalid_model_output";
  if (/no_candidate/.test(value)) return "capacity";
  if (/overloaded|429|rate.limit/.test(value)) return "busy";
  if (/timed? ?out|timeout|deadline/.test(value)) return "timeout";
  if (/protocol/.test(value)) return "protocol";
  if (/unavailable|502|503|connection|offline/.test(value)) return "unavailable";
  return "other";
}
export function maintenanceFailureLabel(kind, language = "en") {
  return (LABELS[kind] || LABELS.other)[language === "zh" ? 0 : 1];
}
