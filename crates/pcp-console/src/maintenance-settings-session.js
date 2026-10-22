// Owns the remote settings draft, request lifetime and uncertain-save recovery.
const fingerprint = value => JSON.stringify(value, (_, item) => item && typeof item === "object" && !Array.isArray(item)
  ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);

export class MaintenanceSettingsSession {
  constructor({ request, mutate, readDraft, populate, snapshotDraft, changed = () => {} }) {
    Object.assign(this, { request, mutate, readDraft, populate, snapshotDraft, changed });
    this.active = false;
    this.generation = 0;
    this.reset();
  }
  reset() {
    this.snapshot = null;
    this.baseline = null;
    this.loading = this.saving = this.uncertain = false;
    this.error = null;
    this.notice = "";
    this.pending = null;
  }
  get loaded() { return this.snapshot !== null; }
  get dirty() { return this.loaded && this.baseline !== fingerprint(this.readDraft()); }
  get protected() { return this.dirty || this.saving || this.uncertain; }
  open() { this.active = true; }
  suspend() {
    this.active = false;
    this.generation++;
    this.loading = false;
    this.pending = null;
    this.changed();
  }
  close() { this.active = false; this.generation++; this.reset(); this.changed(); }
  touch() { this.notice = ""; this.changed(); }
  async load({ reconcile = false } = {}) {
    if (this.saving || !this.active) return null;
    if (this.pending) return this.pending;
    const generation = ++this.generation;
    this.loading = true;
    if (!this.uncertain) this.error = null;
    this.changed();
    const pending = (async () => {
      try {
        const snapshot = await this.request("/api/maintenance");
        if (!this.active || generation !== this.generation) return null;
        // An edit made while a request was running takes precedence over it.
        const preserve = this.dirty || this.uncertain;
        const baseline = fingerprint(this.snapshotDraft(snapshot));
        this.snapshot = snapshot;
        this.baseline = baseline;
        if (!preserve) this.populate(snapshot);
        if (reconcile) {
          this.uncertain = false;
          this.error = null;
          this.notice = this.dirty ? "Saved settings refreshed; your draft is kept. Review it before saving again." : "Current settings match your draft.";
        }
        return snapshot;
      } catch (error) {
        if (this.active && generation === this.generation) this.error = error;
        return null;
      } finally {
        if (this.active && generation === this.generation) {
          this.loading = false;
          this.pending = null;
          this.changed();
        }
      }
    })();
    this.pending = pending;
    return pending;
  }
  async save() {
    if (!this.loaded || this.saving || this.loading || this.uncertain || !this.dirty) return false;
    const draft = structuredClone(this.readDraft());
    this.saving = true;
    this.error = null;
    this.notice = "";
    this.changed();
    try {
      await this.mutate("/api/maintenance/settings", draft);
      this.baseline = fingerprint(draft);
      this.notice = "Saved";
      return true;
    } catch (error) {
      this.error = error;
      this.uncertain = error.mutationOutcome !== "not_applied";
      return false;
    } finally {
      this.saving = false;
      this.changed();
    }
  }
}
