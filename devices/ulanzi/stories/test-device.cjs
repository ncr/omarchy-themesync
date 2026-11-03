const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const renderer=fs.readFileSync(__dirname+'/renderer.js','utf8'),app=fs.readFileSync(__dirname+'/device.js','utf8');
function boot(saved=null){let t=0,day='2026-11-03',state=saved,calls=[],saves=0;const c=vm.createContext({nativeTime:()=>t,nativeDay:()=>day,nativeLoad:()=>state,nativeSave:s=>(state=JSON.parse(s),saves++,true),nativeApi:(...a)=>(calls.push(a),true),nativeInfo:()=>({frames:0})});vm.runInContext(renderer,c);vm.runInContext(app,c);return {c,advance:n=>t+=n,date:s=>day=s,req:(...a)=>c.storyRequest(...a),frame:()=>c.storyTick(),state:()=>state,saves:()=>saves,calls};}
let d=boot();assert.equal(d.req('GET','/status',{}).count,0);
let base={day:'2026-11-03',count:12,revision:1,animate:false};d.req('PUT','/state',base);assert.equal(d.req('GET','/status',{}).animation,null);
assert.equal(d.frame().length,2496);
for(let i=0;i<6;i++){const expected=['rail','orbit','vinyl','maker','garden','city'][i];assert.equal(d.req('GET','/status',{}).scene,expected);d.req('POST','/input',{control:'right',event:'click'});}
d.req('PUT','/state',{...base,count:14,revision:2,animate:true});assert.equal(d.req('GET','/status',{}).animation.ordinal,13);assert.equal(d.req('GET','/status',{}).queue,1);
d.req('PUT','/state',{...base,count:14,revision:2,animate:true});assert.equal(d.req('GET','/status',{}).queue,1);
d.advance(33);d.frame();assert.equal(d.req('GET','/status',{}).animation.ordinal,14);
d.advance(33);d.frame();assert.equal(d.req('GET','/status',{}).animation,null);
d.req('POST','/replay',{});d.advance(33);d.frame();assert.equal(d.req('GET','/status',{}).count,14);
let reboot=boot(d.state());assert.equal(reboot.req('GET','/status',{}).count,14);assert.equal(reboot.req('GET','/status',{}).animation,null);
reboot.req('POST','/input',{control:'middle',event:'click'});assert.equal(reboot.frame(),null);
reboot.req('PUT','/state',{...base,count:15,revision:3,animate:true});assert.equal(reboot.frame().length,2496);reboot.advance(33);assert.equal(reboot.frame(),null);
reboot.date('2026-11-04');reboot.frame();assert.equal(reboot.req('GET','/status',{}).count,0);
assert.throws(()=>reboot.req('PUT','/state',{...base,count:100,revision:4}));
for(const id of ['city','rail','orbit','vinyl','maker','garden']){let b=boot();b.req('POST','/scene',{scene:id});b.req('PUT','/state',{...base,count:20,revision:1});const before=b.frame();b.req('PUT','/state',{...base,count:21,revision:2,animate:true});assert.equal(b.req('GET','/status',{}).count,21);assert.notDeepEqual(Array.from(b.frame()),Array.from(before));b.advance(61);assert.equal(b.req('GET','/status',{}).count,21);}
let themed=boot();themed.req('PUT','/palette',{accent:'00ffff',name:'test',secondary:'ff0080'});let writes=themed.saves();themed.req('PUT','/palette',{name:'test',secondary:'ff0080',accent:'00ffff'});assert.equal(themed.saves(),writes);
console.log('Device: six scenes, controls, baseline, burst queue, idempotence, reboot, clock alert return, midnight, overflow: PASS');
