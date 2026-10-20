// Dynamic maintenance deployment metadata. Discovery never changes saved routes.
const ALL_EFFORTS = ["none", "low", "medium", "high", "xhigh", "max", "ultra"];
export function modelEffortsForDeployment(id, catalog = null) {
  const model = catalog?.models?.find(model => model.deployment_id === id);
  return {
    efforts: model ? ALL_EFFORTS.filter(effort => model.supported_efforts.includes(effort)) : [...ALL_EFFORTS],
    verified: Boolean(model && catalog.available && !catalog.stale && !model.observation_stale),
  };
}
export function modelDeploymentOptions(catalog, intent, current = "") {
  const options = (catalog?.models || [])
    .filter(model => ["advanced", "exceptional"].includes(model.intents?.[intent]) && model.supported_efforts?.length)
    .map(model => ({ id: model.deployment_id, configured: false,
      unavailable: model.availability === "unavailable",
      stale: Boolean(catalog.stale || model.observation_stale),
    }));
  if (current && !options.some(option => option.id === current)) {
    options.push({ id: current, configured: true, unavailable: false, stale: true });
  }
  return options;
}
export function modelDeploymentLabel(id) {
  const match = /^codex_gpt_(\d+(?:_\d+)*)_(luna|sol|terra|astra)$/.exec(id);
  return match ? `GPT-${match[1].replaceAll("_", ".")} ${match[2][0].toUpperCase()}${match[2].slice(1)}` : id;
}
export function createModelCatalog({ request, changed }) {
  let snapshot = { models: [], available: false, stale: false };
  let pending = null;
  return {
    get snapshot() { return snapshot; },
    refresh({ force = false } = {}) {
      if (pending) return pending;
      pending = (async () => {
        try {
          const next = await request(`/api/maintenance/models${force ? "?refresh=true" : ""}`);
          if (!next || !Array.isArray(next.models)) throw new Error("Invalid model catalog");
          snapshot = next;
        } catch {
          snapshot = { ...snapshot, stale: snapshot.available, error: "unavailable" };
        }
        changed(snapshot);
        return snapshot;
      })().finally(() => { pending = null; });
      return pending;
    },
  };
}
