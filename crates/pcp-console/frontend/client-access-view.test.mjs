import test from "node:test";
import assert from "node:assert/strict";
import {createClientAccessView} from "../src/client-access-view.js";

class Node {
  constructor(tag = "div", text = "") { this.tagName=tag.toUpperCase(); this.textContent=text; this.children=[]; this.dataset={}; this.handlers={}; this.attributes={}; this.open=false; }
  append(...nodes) {this.children.push(...nodes);}
  replaceChildren(...nodes) {this.children=nodes;}
  setAttribute(key,value) {this.attributes[key]=value;}
  addEventListener(key,value) {this.handlers[key]=value;}
  contains(node) {return this===node || this.children.some(child=>child.contains(node));}
  querySelectorAll(selector) {
    const matches=node=>selector==="button" ? node.tagName==="BUTTON"
      : selector==="details[data-client-disclosure]" ? node.tagName==="DETAILS" && node.dataset.clientDisclosure!==undefined
      : selector==="[data-client-focus]" && node.dataset.clientFocus!==undefined;
    return this.children.flatMap(child=>[...(matches(child)?[child]:[]),...child.querySelectorAll(selector)]);
  }
  focus() {document.activeElement=this;}
  showModal() {this.open=true;}
}
const text=node=>[node.textContent,...node.children.map(text)].join(" ");
const client=(overrides={})=>({registration_id:"reg_one",client:{principal:{principalId:"client:one",displayName:"Client One"}},approved_access:{mode:"contribute",scopes:["user:self"],read_all_scopes:true,allow_cross_scope_derivation:false},project_scopes:["project:a","project:b"],allow_project_registration:true,created_at:"2026-10-01",last_opened_at:"2026-10-20",...overrides});
const snapshot=(registrations=[client()],pending=[])=>({result:{status:"snapshot",registrations,pending}});
function fixture({api=async()=>snapshot(),mutate=async()=>{},confirmAction=async()=>false}={}) {
  const nodes=new Map(),errors=[];
  const byId=id=>{if(!nodes.has(id))nodes.set(id,new Node());return nodes.get(id);};
  globalThis.document={activeElement:null,querySelector:()=>byId("enrollment-dialog").open?byId("enrollment-dialog"):null};
  const view=createClientAccessView({api,mutate,confirmAction,byId,element:(tag,cls,value)=>Object.assign(new Node(tag,value),{className:cls}),t:value=>value,scopeName:scope=>({"project:a":"Project A","project:b":"Project B"})[scope]||scope,formatTime:String,showError:error=>errors.push(error)});
  return {view,byId,errors};
}

test("client configuration separates permission policy and named projects from connection identifiers",async()=>{
  const {view,byId}=fixture();await view.load();
  const row=byId("enrollment-registered").children[0];
  assert.match(text(row.children[0]),/Client One.*Contribute memories.*Reads all scopes.*2 writable projects/);
  assert.doesNotMatch(text(row.children[0]),/client:one|reg_one|project:/);
  const permissions=row.children[1].children[0];
  assert.equal(permissions.open,false);
  assert.match(text(permissions),/Read all scopes; other permissions remain scoped/);
  assert.match(text(permissions),/Personal memory/);
  assert.match(text(permissions),/Project A.*Project B/);
  assert.match(text(permissions),/Cross-scope derivation.*Not allowed/);
  assert.match(text(row.children[1].children[1]),/client:one.*reg_one/);
});

test("pending requests expose their requested authority and do not imply global read access",async()=>{
  const pending={request_id:"request_one",client:client().client,requested_access:{mode:"observe",scopes:["project:a"],read_all_scopes:false},requested_at:"2026-10-20",expires_at:"2026-10-21"};
  const {view,byId}=fixture({api:async()=>snapshot([],[pending])});await view.load();
  const row=byId("enrollment-pending").children[0];
  assert.equal(row.children[1].children[0].open,true);
  assert.match(text(row),/Requested permissions.*Permissions apply to listed scopes/);
  assert.doesNotMatch(text(row.children[0]),/Reads all scopes|Read memories/);
  assert.equal(byId("enrollment-badge").textContent,"1");
  assert.equal(byId("enrollment-dialog").open,true);
});

test("polling preserves an expanded permission view and keyboard focus, including a changed snapshot",async()=>{
  let updated="2026-10-20";
  const {view,byId}=fixture({api:async()=>snapshot([client({last_opened_at:updated})])});
  await view.load();
  const root=byId("enrollment-registered"),original=root.children[0],details=original.children[1].children[0];
  details.open=true;details.children[0].focus();
  await view.load();
  assert.equal(root.children[0],original);
  assert.equal(document.activeElement,details.children[0]);
  updated="2026-10-21";await view.load();
  assert.notEqual(root.children[0],original);
  assert.equal(root.children[0].children[1].children[0].open,true);
  assert.equal(document.activeElement.dataset.clientFocus,"permissions:reg_one");
});

test("a stale enrollment response cannot replace a newer registration snapshot",async()=>{
  const pending=[];
  const {view,byId}=fixture({api:()=>new Promise(resolve=>pending.push(resolve))});
  const old=view.load(),current=view.load();
  pending[1](snapshot([client({client:{principal:{principalId:"client:new",displayName:"New client"}}})]));await current;
  pending[0](snapshot());await old;
  assert.match(text(byId("enrollment-registered")),/New client/);
  assert.doesNotMatch(text(byId("enrollment-registered")),/Client One/);
});

test("revocation waits for explicit confirmation and polling cannot interrupt the operator mutation",async()=>{
  let confirm=false,resolveMutation,loads=0;const writes=[];
  const {view,byId}=fixture({api:async()=>{loads++;return snapshot();},confirmAction:async()=>confirm,mutate:(...args)=>{writes.push(args);return new Promise(resolve=>{resolveMutation=resolve;});}});
  await view.load();
  const root=byId("enrollment-registered"),revoke=root.querySelectorAll("button").find(button=>button.textContent==="Revoke");
  await revoke.handlers.click();assert.equal(writes.length,0);
  confirm=true;const action=revoke.handlers.click();await new Promise(resolve=>setImmediate(resolve));
  assert.equal(writes[0][0],"/api/enrollment/registrations/reg_one/revoke");
  assert.ok(root.querySelectorAll("button").every(button=>button.disabled));
  await view.load();assert.equal(loads,1);
  resolveMutation();await action;
  assert.equal(loads,2);
  assert.ok(root.querySelectorAll("button").every(button=>!button.disabled));
});
