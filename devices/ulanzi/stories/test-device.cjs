const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const renderer=fs.readFileSync(__dirname+'/renderer.js','utf8'),app=fs.readFileSync(__dirname+'/device.js','utf8');
const ids=['hours','sky','jar','board','glass'];
function boot(saved=null){let t=0,day='2026-11-03',wall=8*3600,state=saved,calls=[],saves=0;
 const FakeDate=function(){const s=wall;return {getHours:()=>Math.floor(s/3600),getMinutes:()=>Math.floor(s/60)%60,getSeconds:()=>Math.floor(s)%60,getMilliseconds:()=>0};};
 const c=vm.createContext({Date:FakeDate,nativeTime:()=>t,nativeDay:()=>day,nativeLoad:()=>state,nativeSave:s=>(assert(s.length<=8192,'state must fit storyd'),state=JSON.parse(s),saves++,true),nativeApi:(...a)=>(calls.push(a),true),nativeInfo:()=>({frames:0})});
 vm.runInContext(renderer,c);vm.runInContext(app,c);
 return {c,advance:n=>t+=n,date:s=>day=s,clock:s=>wall=s,req:(...a)=>c.storyRequest(...a),frame:()=>c.storyTick(),state:()=>state,saves:()=>saves,calls};}
const orders=n=>Array.from({length:n},(_,i)=>[3600+i*200,i%7?20+i*3:null]);
let d=boot();assert.equal(d.req('GET','/status',{}).count,0);
let base={day:'2026-11-03',count:12,orders:orders(12),revision:1,animate:false};d.req('PUT','/state',base);assert.equal(d.req('GET','/status',{}).animation,null);
assert.equal(d.frame().length,2496);
for(let i=0;i<5;i++){assert.equal(d.req('GET','/status',{}).scene,ids[i]);d.req('POST','/input',{control:'right',event:'click'});}
d.req('PUT','/state',{...base,count:14,orders:orders(14),revision:2,animate:true});assert.equal(d.req('GET','/status',{}).animation.ordinal,13);assert.equal(d.req('GET','/status',{}).queue,1);
d.req('PUT','/state',{...base,count:14,orders:orders(14),revision:2,animate:true});assert.equal(d.req('GET','/status',{}).queue,1);
d.advance(7);d.frame();assert.equal(d.req('GET','/status',{}).animation.ordinal,14);
d.advance(7);d.frame();assert.equal(d.req('GET','/status',{}).animation,null);
d.req('POST','/replay',{});assert.equal(d.req('GET','/status',{}).animation.ordinal,14);d.advance(7);d.frame();assert.equal(d.req('GET','/status',{}).count,14);
let reboot=boot(d.state());assert.equal(reboot.req('GET','/status',{}).count,14);assert.equal(reboot.req('GET','/status',{}).orders_known,14);assert.equal(reboot.req('GET','/status',{}).animation,null);
reboot.req('POST','/input',{control:'middle',event:'click'});assert.equal(reboot.frame(),null);
reboot.req('PUT','/state',{...base,count:15,orders:orders(15),revision:3,animate:true});assert.equal(reboot.frame().length,2496);reboot.advance(7);assert.equal(reboot.frame(),null);
reboot.date('2026-11-04');reboot.frame();assert.equal(reboot.req('GET','/status',{}).count,0);assert.equal(reboot.req('GET','/status',{}).orders_known,0);
// Bad input is refused: wrong length, unsorted, out-of-day times.
for(const bad of [orders(3),[[5,1],[4,1],[6,1],[7,1]],[[90000,1],[1,1],[2,1],[3,1]]])assert.throws(()=>reboot.req('PUT','/state',{day:'2026-11-04',count:4,orders:bad,revision:9}));
// A bridge that sends only the count: arrivals are placed at the clock's current time.
let plain=boot();plain.clock(15*3600+120);plain.req('PUT','/state',{day:'2026-11-03',count:2,revision:1,animate:true});assert.deepEqual(plain.state().orders,[[54120,null],[54120,null]]);
// Replay with nothing paid today does nothing; the full 100-order state still fits in storyd's 8 KiB.
let empty=boot();empty.req('POST','/replay',{});assert.equal(empty.req('GET','/status',{}).animation,null);
let full=boot();full.req('PUT','/state',{day:'2026-11-03',count:450,orders:Array.from({length:100},(_,i)=>[i*840,999999]),revision:1});for(const id of ids){full.req('POST','/scene',{scene:id});assert.equal(full.frame().length,2496);}
let themed=boot();themed.req('PUT','/palette',{accent:'00ffff',name:'test',secondary:'ff0080'});let writes=themed.saves();themed.req('PUT','/palette',{name:'test',secondary:'ff0080',accent:'00ffff'});assert.equal(themed.saves(),writes);
console.log('Device: five scenes, controls, burst queue, idempotence, reboot, clock alert return, midnight, order validation, count-only bridge, 100-order state: PASS');
