const fs=require('node:fs'),assert=require('node:assert/strict'),vm=require('node:vm'),crypto=require('node:crypto');
const demo=require('./demos.js'),base=__dirname+'/';
const ids=demo.catalogue.map(x=>x.id);
assert.deepEqual(ids,['pendulums','fireflies','harmonograph','ripples','aurora','bloom','lace','tide']);
const bloom={accent:'4cc9ff',secondary:'b97aff'},jade={accent:'509475',secondary:'d2689c',colors:{foreground:'c1c497',yellow:'459451',orange:'a2734b'}};
const hash=a=>crypto.createHash('sha256').update(Buffer.from(demo.rgb(a))).digest('hex'),stats=[];
for(const m of demo.catalogue){
 // Seamless: the loop's end is its beginning.
 for(const t of [0,.4,3.3]){const x=demo.rgb(demo.frame(m.id,t,bloom)),y=demo.rgb(demo.frame(m.id,t+m.duration,bloom));assert(x.every((v,i)=>Math.abs(v-y[i])<=2),m.id+' must loop at its duration');}
 let unique=new Set(),min=832,max=0;
 for(let i=0;i<m.duration*8;i++){const a=demo.frame(m.id,i/8,bloom);assert.equal(a.length,832);let lit=0;for(const c of a){assert(Number.isInteger(c)&&c>=0&&c<=0xffffff);if(c)lit++;}min=Math.min(min,lit);max=Math.max(max,lit);unique.add(hash(a));}
 assert(max>12&&max<760,m.id+': empty or flooded');assert(unique.size>=m.duration*4,m.id+' barely animates');
 assert.notEqual(hash(demo.frame(m.id,5,bloom)),hash(demo.frame(m.id,5,jade)),m.id+': theme must change the picture');
 stats.push({id:m.id,unique:unique.size,lit:[min,max]});
}
assert.notDeepEqual(demo.frame('pendulums',0,bloom,{title:true}),demo.frame('pendulums',0,bloom),'the title card comes first');
function boot(saved=null){let t=0,state=saved,writes=0;let c=vm.createContext({nativeTime:()=>t,nativeDay:()=>'2026-11-03',nativeLoad:()=>state,nativeSave:s=>(state=JSON.parse(s),writes++,true),nativeApi:()=>true,nativeInfo:()=>({})});for(const p of [base+'renderer.js',base+'demos.js',base+'device.js'])vm.runInContext(fs.readFileSync(p,'utf8'),c);return {req:(...a)=>c.storyRequest(...a),tick:()=>c.storyTick(),step:s=>{for(let i=0;i<s*8;i++){t+=.125;c.storyTick();}},state:()=>state,writes:()=>writes};}
let b=boot();b.req('PUT','/state',{day:'2026-11-03',count:12,revision:1,animate:false});b.req('POST','/demo',{id:ids[0],autoplay:true});let w=b.writes();b.step(demo.catalogue[0].duration+1.3);assert.equal(b.req('GET','/status',{}).compo.id,ids[1]);assert.equal(b.writes(),w,'autoplay must not write flash');assert.equal(b.req('GET','/status',{}).count,12);
for(const id of ids){b.req('POST','/demo',{id,autoplay:false});assert.equal(b.tick().length,2496);b.step(2);assert.equal(b.req('GET','/status',{}).compo.id,id);}
b.req('POST','/input',{control:'right',event:'click'});assert.equal(b.req('GET','/status',{}).compo.id,ids[0]);b.req('POST','/input',{control:'left',event:'click'});assert.equal(b.req('GET','/status',{}).compo.id,ids.at(-1));
let restored=boot(b.state());assert.equal(restored.req('GET','/status',{}).mode,'compo');assert.equal(restored.req('GET','/status',{}).count,12);
for(const mode of ['stories','clock','compo']){restored.req('POST','/input',{control:'middle',event:'click'});assert.equal(restored.req('GET','/status',{}).mode,mode);}
restored.req('PUT','/state',{day:'2026-11-03',count:13,revision:2,animate:true});assert(restored.req('GET','/status',{}).animation);restored.step(8);assert.equal(restored.req('GET','/status',{}).mode,'compo');assert.equal(restored.req('GET','/status',{}).animation,null);assert.equal(restored.req('GET','/status',{}).count,13);
restored.req('POST','/scene',{scene:'hours'});assert.equal(restored.req('GET','/status',{}).mode,'stories');assert.throws(()=>restored.req('POST','/demo',{id:'missing'}));
console.log(JSON.stringify({compo_loops:stats,controls_persistence_and_real_order_return:true}));
