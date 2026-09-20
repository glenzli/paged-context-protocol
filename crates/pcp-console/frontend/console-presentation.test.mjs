import test from 'node:test';
import assert from 'node:assert/strict';
import {operationLabel, kindLabel, technicalDetails} from '../src/console-presentation.js';

test('display names describe known work without guessing at unknown operation contracts',()=>{
  const localized=key=>({'Organize candidates':'整理候选','Reviewed memory':'经审阅的记忆'})[key]||key;
  assert.equal(operationLabel('organize_candidates',localized),'整理候选');
  assert.equal(kindLabel('reviewed_capture',localized),'经审阅的记忆');
  assert.equal(operationLabel('custom_worker_v2',localized),'custom_worker_v2');
  assert.equal(kindLabel('custom_kind',localized),'custom_kind');
});

test('technical disclosure preserves exact identifiers and zero values while omitting missing fields',()=>{
  const node=(tag,cls,text)=>({tag,className:cls,textContent:text,children:[],append(...items){this.children.push(...items);}});
  const detail=technicalDetails(node,'Details',[['Revision','rev_exact_123'],['Count',0],['Missing',null]]);
  assert.equal(detail.tag,'details');
  assert.equal(detail.children[0].textContent,'Details');
  assert.deepEqual(detail.children[1].children.map(n=>n.textContent),['Revision','rev_exact_123','Count','0']);
});

test('late scope names update only label text and preserve unresolved identifiers', async()=>{
  const {scopeLabel,refreshScopeLabels}=await import('../src/console-presentation.js');
  const node=(tag,cls,text)=>({tag,className:cls,textContent:text,attributes:{},setAttribute(k,v){this.attributes[k]=v;},getAttribute(k){return this.attributes[k];}});
  const label=scopeLabel(node,['scope:known','scope:unknown'],value=>value,'td');
  assert.equal(label.textContent,'scope:known, scope:unknown');
  const root={querySelectorAll:()=>[label]};
  refreshScopeLabels(root,value=>value==='scope:known'?'Personal memory':value);
  assert.equal(label.tag,'td');
  assert.equal(label.textContent,'Personal memory, scope:unknown');
  assert.equal(label.getAttribute('data-scope-names'),'["scope:known","scope:unknown"]');
});
