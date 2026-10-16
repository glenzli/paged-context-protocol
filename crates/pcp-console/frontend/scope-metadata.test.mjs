import test from 'node:test';
import assert from 'node:assert/strict';
import {createScopeMetadataRefresh} from '../src/scope-metadata.js';

test('newly appearing project refreshes names before the periodic interval, with one in-flight request',async()=>{
  let scopes=[{namespace:'project:old',displayName:'Old'}], now=100, calls=0, release;
  const cache=createScopeMetadataRefresh({getScopes:()=>scopes,clock:()=>now,load:()=>{calls++;return new Promise(resolve=>{release=resolve;});},apply:value=>{scopes=value;}});
  cache.markFresh();
  await cache.refresh(['project:old']);
  assert.equal(calls,0);
  const a=cache.refresh(['project:new']);
  const b=cache.refresh(['project:new']);
  assert.equal(a,b);
  await Promise.resolve();
  release([...scopes,{namespace:'project:new',displayName:'New'}]);
  await a;
  assert.equal(calls,1);
  assert.equal(scopes.find(s=>s.namespace==='project:new').displayName,'New');
  await cache.refresh(['project:new']);
  assert.equal(calls,1);
});

test('periodic metadata refresh discovers empty projects and failed loads preserve names with bounded retries',async()=>{
  let scopes=[{namespace:'project:known'}], now=0, calls=0;
  const cache=createScopeMetadataRefresh({getScopes:()=>scopes,clock:()=>now,load:async()=>{calls++;if(calls===1)throw Error('offline');return [...scopes,{namespace:'project:empty'}];},apply:value=>{scopes=value;}});
  cache.markFresh(); now=30000;
  await assert.rejects(cache.refresh(),/offline/);
  assert.equal(scopes.length,1);
  await cache.refresh(['project:unknown']);
  assert.equal(calls,1);
  now+=10000;
  await cache.refresh();
  assert.equal(scopes.length,2);
});
