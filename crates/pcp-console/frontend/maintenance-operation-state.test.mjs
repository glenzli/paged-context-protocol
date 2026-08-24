import assert from "node:assert/strict";
import test from "node:test";

import {
  MAINTENANCE_OWNER,
  maintenanceControllerOwner,
  maintenanceTargetBlocked,
} from "../src/maintenance-operation-state.js";

test("the active maintenance workflow visibly owns the shared controller", () => {
  assert.equal(maintenanceControllerOwner({ convergenceActive: true }), MAINTENANCE_OWNER.CONVERGENCE);
  assert.equal(maintenanceControllerOwner({ manualSessionActive: true }), MAINTENANCE_OWNER.MANUAL);
  assert.equal(maintenanceControllerOwner({ archiveSessionActive: true }), MAINTENANCE_OWNER.ARCHIVE);
  assert.equal(maintenanceControllerOwner({ automationState: "running" }), MAINTENANCE_OWNER.AUTOMATIC);
  assert.equal(maintenanceControllerOwner(), null);
});

test("one workflow blocks only competing maintenance entry points", () => {
  assert.equal(maintenanceTargetBlocked(MAINTENANCE_OWNER.ARCHIVE, MAINTENANCE_OWNER.MANUAL), true);
  assert.equal(maintenanceTargetBlocked(MAINTENANCE_OWNER.ARCHIVE, MAINTENANCE_OWNER.CONVERGENCE), true);
  assert.equal(maintenanceTargetBlocked(MAINTENANCE_OWNER.ARCHIVE, MAINTENANCE_OWNER.ARCHIVE), false);
  assert.equal(maintenanceTargetBlocked(null, MAINTENANCE_OWNER.MANUAL), false);
});
