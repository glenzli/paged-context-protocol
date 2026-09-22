export const MODEL_ROUTE_GROUPS = [
  { id: "summary", operations: ["summarize_page", "summarize_pages"] },
  { id: "organization", operations: ["organize_candidates", "select_packing", "analyze_packing"] },
  { id: "knowledge", operations: ["select_relation", "extract_topic"] },
  { id: "verification", operations: ["verify_maintenance", "assess_archive", "select_retention_milestones"] },
  { id: "feedback", operations: ["reconcile_feedback", "review_update"] },
];

const ALL_MODEL_EFFORTS = ["none", "low", "medium", "high", "xhigh", "max", "ultra"];
const INFER_DEPLOYMENT_EFFORTS = {
  codex_gpt_6_luna: ["low", "medium", "high", "xhigh", "max"],
  codex_gpt_6_sol: ["low", "medium", "high", "xhigh", "max", "ultra"],
  codex_gpt_5_6_luna: ["low", "medium", "high", "xhigh", "max"],
  codex_gpt_5_6_terra: ["low", "medium", "high", "xhigh", "max", "ultra"],
  codex_gpt_5_6_sol: ["low", "medium", "high", "xhigh", "max", "ultra"],
};

// Known local Infer deployments. Custom IDs stay editable because Console
// cannot read the operator-only Infer provider catalog.
export function modelEffortsForDeployment(deploymentId) {
  const known = Object.hasOwn(INFER_DEPLOYMENT_EFFORTS, deploymentId)
    ? INFER_DEPLOYMENT_EFFORTS[deploymentId] : null;
  return { efforts: known || ALL_MODEL_EFFORTS, verified: Boolean(known) };
}

function sameRoute(left, right) {
  return left.deployment_id === right.deployment_id && left.effort === right.effort;
}

export function summarizeModelRouteGroups(routes, configuredGroups = {}, configuredOverrides = {}) {
  return MODEL_ROUTE_GROUPS.map((group) => {
    const available = group.operations.filter((operation) => routes[operation]);
    const counts = new Map();
    for (const operation of available) {
      const route = routes[operation];
      const key = JSON.stringify([route.deployment_id, route.effort]);
      counts.set(key, (counts.get(key) || 0) + 1);
    }
    const defaultOperation = available.reduce((best, operation) => {
      if (!best) return operation;
      const key = (name) => JSON.stringify([routes[name].deployment_id, routes[name].effort]);
      return counts.get(key(operation)) > counts.get(key(best)) ? operation : best;
    }, null);
    const defaultRoute = configuredGroups[group.id]
      ? { ...configuredGroups[group.id] }
      : defaultOperation ? { ...routes[defaultOperation] } : null;
    const overrides = Object.fromEntries(available
      .filter((operation) => configuredGroups[group.id]
        ? Object.hasOwn(configuredOverrides, operation)
        : !sameRoute(routes[operation], defaultRoute))
      .map((operation) => [operation, { ...routes[operation] }]));
    return { ...group, defaultRoute, overrides };
  });
}

export function resolveModelRouteGroups(originalRoutes, groups) {
  const routes = { ...originalRoutes };
  for (const group of groups) {
    for (const operation of group.operations) {
      if (!originalRoutes[operation]) continue;
      routes[operation] = { ...(group.overrides[operation] || group.defaultRoute) };
    }
  }
  return routes;
}
