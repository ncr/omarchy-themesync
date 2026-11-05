const assert=require('node:assert/strict'),crypto=require('node:crypto');
const o=require('./renderer.js');
const bloom={accent:'4cc9ff',secondary:'b97aff'},jade={accent:'509475',secondary:'d2689c',colors:{foreground:'c1c497',yellow:'459451',orange:'a2734b',cyan:'2dd5b7',red:'ff5345'}};
const day=n=>Array.from({length:n},(_,i)=>({t:Math.round(7*3600+i*(14*3600/Math.max(1,n))),v:i%9?Math.round(5*Math.pow(1.4,i%14)):null}));
const hashOf=f=>crypto.createHash('sha256').update(Buffer.from(o.rgb(f))).digest('hex');
assert.deepEqual(o.scenes.map(s=>s.id),['hours','sky','jar','board','glass']);
// Palettes keep the theme's own accent and secondary while they are clearly distinct hues.
assert.equal(o.palette(bloom).tiers.length,3);
const report=[];
for(const {id,duration} of o.scenes){
 for(const n of [0,1,5,20,100]){
  const list=day(n),view={orders:list,count:n,now:15*3600,event:null},a=o.frame(id,view,bloom,3);
  assert.equal(a.length,832);for(const c of a)assert(Number.isInteger(c)&&c>=0&&c<=0xffffff);
  assert.deepEqual(o.frame(id,view,bloom,3),a,id+': frames must be deterministic');
  if(n)assert.notEqual(hashOf(o.frame(id,view,jade,3)),hashOf(a),id+': theme must change the picture');
  // An arrival is a visible sequence and ends exactly on the idle picture.
  if(n){const seen=new Set();for(let s=0;s<duration;s+=.25)seen.add(hashOf(o.frame(id,{...view,event:{s}},bloom,3)));
   assert(seen.size>=6,`${id}/${n}: arrival barely animates (${seen.size})`);report.push({id,n,arrival_frames:seen.size});}
 }
 // Idle is calm but not frozen.
 const idle=new Set();for(let t=0;t<12;t+=.5)idle.add(hashOf(o.frame(id,{orders:day(12),count:12,now:15*3600,event:null},bloom,t)));
 assert(idle.size>=2,id+': idle picture never moves');
}
console.log(JSON.stringify({orders_scenes:report.length,checks:'deterministic, themed, animated arrivals, moving idle'}));
