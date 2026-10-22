import test from 'node:test';
import assert from 'node:assert/strict';
import {maintenanceFailureKind,maintenanceFailureLabel} from '../src/maintenance-diagnostics.js';

test('execution, unanswered transport, interrupted clock and rejected model output stay distinct',()=>{
  for (const [reason,kind] of [
    ['Infer Runtime maintenance submission timed out: deadline has elapsed','response_timeout'],
    ['Infer Runtime returned HTTP 504 with code `deadline_exceeded`','inference_timeout'],
    ['Infer Runtime maintenance response wait timed out after 130s','response_timeout'],
    ['Infer Runtime maintenance wait interrupted: possible system sleep','execution_interrupted'],
    ['decode strict PCP maintenance decision from Infer Runtime: duplicate field `action`','invalid_model_output'],
    ['serverOverloaded','busy'],['no_candidate','capacity'],['upstream_protocol','protocol'],
  ]) assert.equal(maintenanceFailureKind(reason),kind);
  assert.equal(maintenanceFailureKind('deadline has elapsed','invalid_model_output'),'invalid_model_output');
  assert.equal(maintenanceFailureKind('unknown future cause','future_v2'),'other');
  assert.match(maintenanceFailureLabel('response_timeout','zh'),/未确认/);
  assert.match(maintenanceFailureLabel('execution_interrupted','en'),/possible/);
});

test('local and retained budget denials are waiting, while unknown execution keeps its boundary',()=>{
  for (const reason of [
    'Waiting for maintenance worker: PCP total maintenance budget exhausted; waiting for rolling budget release',
    'Rolling review budget exhausted; waiting for reservations to settle or expire',
    'Another upgraded review has an unsettled reservation',
  ]) assert.equal(maintenanceFailureKind(reason,'worker_failed'),'waiting_budget');
  assert.equal(maintenanceFailureKind('unused text','waiting_budget'),'waiting_budget');
  assert.equal(maintenanceFailureKind('maintenance response wait timed out; outcome unknown; PCP total maintenance budget exhausted'),'response_timeout');
  assert.equal(maintenanceFailureKind('PCP total maintenance budget exhausted','response_timeout'),'response_timeout');
  assert.equal(maintenanceFailureKind('maintenance wait interrupted; PCP total maintenance budget exhausted'),'execution_interrupted');
  assert.match(maintenanceFailureLabel('waiting_budget','zh'),/预算准入/);
});
