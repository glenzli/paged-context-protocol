export const MAINTENANCE_OWNER = Object.freeze({
  CONVERGENCE: "convergence",
  MANUAL: "manual",
  ARCHIVE: "archive",
  AUTOMATIC: "automatic",
});

export function maintenanceControllerOwner({
  convergenceActive = false,
  convergenceRunning = false,
  manualSessionActive = false,
  manualBusy = false,
  archiveSessionActive = false,
  archiveBusy = false,
  automationState = "not_started",
} = {}) {
  if (convergenceActive || convergenceRunning) return MAINTENANCE_OWNER.CONVERGENCE;
  if (manualSessionActive || manualBusy) return MAINTENANCE_OWNER.MANUAL;
  if (archiveSessionActive || archiveBusy) return MAINTENANCE_OWNER.ARCHIVE;
  if (automationState === "running") return MAINTENANCE_OWNER.AUTOMATIC;
  return null;
}

export function maintenanceTargetBlocked(owner, target) {
  return owner !== null && owner !== target;
}
