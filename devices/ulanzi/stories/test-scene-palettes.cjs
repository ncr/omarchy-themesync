const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(__dirname+'/renderer.js','utf8'),controller=fs.readFileSync(__dirname+'/device.js','utf8');
const bloom={name:'p-bloom',accent:'4cc9ff',secondary:'b97aff'},jade={name:'osaka-jade',accent:'509475',secondary:'d2689c'};
function boot(saved=null){let state=saved,writes=0;const context=vm.createContext({nativeTime:()=>8,nativeDay:()=>'2026-11-03',nativeLoad:()=>state,nativeSave:s=>(state=JSON.parse(s),writes++,true),nativeApi:()=>true,nativeInfo:()=>({})});vm.runInContext(source,context);vm.runInContext(controller,context);return {ctx:context,req:(...a)=>context.storyRequest(...a),frame:()=>Array.from(context.storyTick()),state:()=>state,writes:()=>writes};}
const report=[];
for(const scene of ['city','rail','orbit','vinyl','maker','garden'])for(const count of [0,12,20]){
 const b=boot();b.req('POST','/scene',{scene});b.req('PUT','/state',{day:'2026-11-03',count,revision:1,animate:false});
 b.req('PUT','/palette',bloom);const before=b.frame();b.req('PUT','/palette',jade);const after=b.frame();
 let changed=0;for(let i=0;i<2496;i+=3){const a=before.slice(i,i+3),c=after.slice(i,i+3);assert.equal(a.some(Boolean),c.some(Boolean),'palette must preserve silhouettes and unlit background');if(a.some((v,j)=>v!==c[j]))changed++;for(const channel of c)assert(channel===0||channel>=50,'avoid low-channel LED haze');}
 assert(changed>=10,`${scene}/${count}: theme name changed but visible pixels did not`);
 const status=b.req('GET','/status',{});assert.equal(status.count,count);assert.equal(status.brightness,35);assert.equal(status.theme.name,'osaka-jade');
 assert.equal(status.palette_rendering,'contrast-accents-v1');
 const saved=b.state(),restored=boot(saved);assert.deepEqual(restored.frame(),after,'palette must survive a device restart');
 const writes=b.writes();b.req('PUT','/palette',{secondary:jade.secondary,accent:jade.accent,name:jade.name});assert.equal(b.writes(),writes,'unchanged palette must not write flash');
 report.push({scene,count,changed_pixels:changed});
}
console.log(JSON.stringify({palette_regressions:report}));
