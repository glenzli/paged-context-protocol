// Refresh scope names independently of the expensive overview/integrity check.
export function createScopeMetadataRefresh({ load, apply, getScopes, clock = Date.now, ttlMs = 30000, retryMs = 10000 }) {
  let refreshedAt = -Infinity;
  let attemptedAt = -Infinity;
  let pending = null;
  return {
    markFresh() { refreshedAt = clock(); },
    refresh(namespaces = []) {
      if (pending) return pending;
      const now = clock();
      const known = new Set(getScopes().map(scope => scope.namespace));
      const missing = namespaces.some(namespace => !known.has(namespace));
      if ((!missing && now - refreshedAt < ttlMs) || now - attemptedAt < retryMs) return Promise.resolve();
      attemptedAt = now;
      pending = Promise.resolve().then(load).then(scopes => {
        apply(scopes);
        refreshedAt = clock();
      }).finally(() => { pending = null; });
      return pending;
    },
  };
}
