const fs=require('node:fs'),assert=require('node:assert/strict'),vm=require('node:vm'),crypto=require('node:crypto');
const demo=require('./demos.js'),base=__dirname+'/';
const frames=new Set(),stats=[];let total=0;
assert.equal(demo.catalogue.length,20);assert.equal(new Set(demo.catalogue.map(x=>x.id)).size,20);
for(const m of demo.catalogue){let unique=new Set(),min=832,max=0;for(let i=0;i<m.duration*8;i++){let a=demo.frame(m.id,i/8);assert.equal(a.length,832);let lit=0;for(const c of a){assert(Number.isInteger(c)&&c>=0&&c<=0xffffff);if(c)lit++;}min=Math.min(min,lit);max=Math.max(max,lit);assert(lit>8&&lit<750,`${m.id}: empty or flooded frame`);let hash=crypto.createHash('sha256').update(Buffer.from(demo.rgb(a))).digest('hex');unique.add(hash);frames.add(hash);total++;}assert(unique.size>=20,m.id+' barely animates');stats.push({id:m.id,unique:unique.size,min,max});}
function boot(saved=null){let t=0,state=saved,writes=0;let c=vm.createContext({nativeTime:()=>t,nativeDay:()=>'2026-11-03',nativeLoad:()=>state,nativeSave:s=>(state=JSON.parse(s),writes++,true),nativeApi:()=>true,nativeInfo:()=>({})});for(const p of [base+'renderer.js',base+'demos.js',base+'device.js'])vm.runInContext(fs.readFileSync(p,'utf8'),c);return {req:(...a)=>c.storyRequest(...a),tick:()=>c.storyTick(),step:s=>{for(let i=0;i<s*8;i++){t+=.125;c.storyTick();}},state:()=>state,writes:()=>writes};}
let b=boot();b.req('PUT','/state',{day:'2026-11-03',count:12,revision:1,animate:false});b.req('POST','/demo',{id:'boot',autoplay:true});let w=b.writes();b.step(18);assert.equal(b.req('GET','/status',{}).compo.id,'bonsai');assert.equal(b.writes(),w,'autoplay must not write flash');assert.equal(b.req('GET','/status',{}).count,12);
for(const id of demo.catalogue.map(x=>x.id)){b.req('POST','/demo',{id,autoplay:false});assert.equal(b.tick().length,2496);b.step(2);assert.equal(b.req('GET','/status',{}).compo.id,id);}
b.req('POST','/input',{control:'right',event:'click'});assert.equal(b.req('GET','/status',{}).compo.id,'boot');b.req('POST','/input',{control:'left',event:'click'});assert.equal(b.req('GET','/status',{}).compo.id,'forever');
let restored=boot(b.state());assert.equal(restored.req('GET','/status',{}).mode,'compo');assert.equal(restored.req('GET','/status',{}).count,12);
for(const mode of ['stories','clock','compo']){restored.req('POST','/input',{control:'middle',event:'click'});assert.equal(restored.req('GET','/status',{}).mode,mode);}
restored.req('PUT','/state',{day:'2026-11-03',count:13,revision:2,animate:true});assert(restored.req('GET','/status',{}).animation);restored.step(61);assert.equal(restored.req('GET','/status',{}).mode,'compo');assert.equal(restored.req('GET','/status',{}).animation,null);assert.equal(restored.req('GET','/status',{}).count,13);
restored.req('POST','/scene',{scene:'city'});assert.equal(restored.req('GET','/status',{}).mode,'stories');assert.throws(()=>restored.req('POST','/demo',{id:'missing'}));
for(const id of ['cube','orbit','silk','gears']){
 const smooth=demo.frame(id,3.137),hard=demo.frame(id,3.137,{}, {antialias:false});
 assert.notDeepEqual(smooth,hard,id+': AA must alter real 52×16 pixels');
 assert([...smooth].some(c=>[c>>>16&255,c>>>8&255,c&255].some(v=>v>0&&v<50)),id+': subpixel coverage must survive');
 assert.notDeepEqual(smooth,demo.frame(id,3.157),id+': subpixel motion must animate between 8 FPS steps');
}
const roles={foreground:'abcdef',yellow:'123456',orange:'654321',red:'123abc',green:'abc123',blue:'987654',magenta:'aabbcc',selection:'112233'};
const p=demo.palette({accent:'123456',secondary:'778899',colors:roles});
assert.equal(p.gold,0x123456);assert.equal(p.orange,0x654321);assert.equal(p.green,0xabc123);assert.equal(p.white,0xabcdef);
b=boot();let body={name:'same-accent',accent:'123456',secondary:'778899',colors:roles};b.req('PUT','/palette',body);w=b.writes();b.req('PUT','/palette',{...body,colors:Object.fromEntries(Object.entries(roles).reverse())});assert.equal(b.writes(),w,'palette key order must not write flash');b.req('PUT','/palette',{...body,colors:{...roles,green:'010203'}});assert.equal(b.writes(),w+1,'same-accent colour changes must apply');
console.log(JSON.stringify({frames_verified:total,unique_frames:frames.size,stats,controls_persistence_and_real_order_return:true}));
