import test from "node:test";
import assert from "node:assert/strict";
import {createConsoleNavigation} from "../src/console-navigation.js";

function fixture(hash="#maintenance") {
  const node=name=>({id:`view-${name}`,dataset:{view:name},attrs:{},events:{},classList:{toggle(){}},setAttribute(k,v){this.attrs[k]=v;},addEventListener(k,v){this.events[k]=v;},focus(){this.focused=true;}});
  const names=["overview","maintenance","context-hub"],tabs=names.map(node),views=names.map(node),events={},history=[];
  const host={location:{hash},history:{pushState(_a,_b,hash){history.push(hash);host.location.hash=hash;}},addEventListener(k,v){events[k]=v;}};
  let nav;nav=createConsoleNavigation({tabs,views,host,onNavigate:(name,options)=>nav.select(name,options),onError:e=>{throw e;}});
  return {nav,tabs,views,host,events,history};
}
test("deep links and history select exactly one accessible panel",()=>{
  const f=fixture();assert.equal(f.nav.locationView(),"maintenance");f.nav.select(f.nav.locationView(),{history:false});
  assert.deepEqual(f.views.map(v=>v.hidden),[true,false,true]);assert.deepEqual(f.tabs.map(t=>t.tabIndex),[-1,0,-1]);
  f.nav.select("context-hub");assert.deepEqual(f.history,["#context-hub"]);
  f.host.location.hash="#maintenance";f.events.hashchange();
  assert.equal(f.views[1].hidden,false);assert.equal(f.history.length,1);
  assert.equal(f.tabs[1].attrs["aria-controls"],"view-maintenance");
  assert.equal(f.views[1].attrs["aria-labelledby"],"tab-maintenance");
});
test("keyboard navigation wraps, focuses and unknown locations fall back safely",()=>{
  const f=fixture("#unknown");assert.equal(f.nav.locationView(),"overview");
  f.tabs[0].events.keydown({key:"ArrowLeft",preventDefault(){}});
  assert.equal(f.tabs[2].focused,true);assert.equal(f.host.location.hash,"#context-hub");
  f.tabs[2].events.keydown({key:"Home",preventDefault(){}});assert.equal(f.host.location.hash,"#overview");
});
