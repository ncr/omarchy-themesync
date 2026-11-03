/* Runs identically in native QuickJS and the deterministic Node tests. */
(function(g){
'use strict';
const art=g.UlanziConcepts, ids=art.themes.map(x=>x.id), today=()=>nativeDay();
let saved=nativeLoad()||{}, day=today(), count=0, revision=0, selected=ids.includes(saved.scene)?saved.scene:'rail';
let clock=!!saved.clock, brightness=Number.isInteger(saved.brightness)?saved.brightness:35;
if(saved.day===day){count=Number.isInteger(saved.count)?Math.max(0,saved.count):0;revision=saved.revision||0;}
let lastSync=saved.last_sync||0, queue=[],active=null, visible=count, returnClock=false, clockApplied=false, theme=saved.theme||{};
// Map structural accents to the desktop hue without lighting the black background.
// Saturation and discrete channel levels avoid the panel driver's low-channel haze.
function ledColour(hex,level){
 let rgb=[0,2,4].map(i=>parseInt(hex.slice(i,i+2),16)),hi=Math.max(...rgb),lo=Math.min(...rgb);
 if(hi<16){rgb=[0,192,255];hi=255;lo=0;}
 if(hi-lo>8){rgb=rgb.map(v=>(v-lo)*hi/(hi-lo));}
 const channels=rgb.map(v=>{const n=Math.min(255,Math.round(v/hi*level/32)*32);return n<64?0:n;});
 return (channels[0]<<16)|(channels[1]<<8)|channels[2];
}
function paletteMap(id){
 if(!theme.accent)return null;
 const map=new Map([[art.C.brown,ledColour(theme.accent,96)],[art.C.teal,ledColour(theme.accent,192)]]);
 if(id!=='garden'){
  map.set(art.C.blue,ledColour(theme.accent,128));
  map.set(art.C.cyan,ledColour(theme.accent,255));
  map.set(art.C.violet,ledColour(theme.secondary,255));
 }
 return map;
}
let maps=Object.fromEntries(ids.map(id=>[id,paletteMap(id)]));
function applyPalette(data,id){const map=maps[id];if(map)for(let i=0;i<data.length;i++){const colour=map.get(data[i]);if(colour!==undefined)data[i]=colour;}}
const demos=g.UlanziDemos||null, demoIDs=demos?demos.catalogue.map(s=>s.id):[];
let compo={enabled:!!(saved.compo&&saved.compo.enabled&&demos),id:saved.compo&&demoIDs.includes(saved.compo.id)?saved.compo.id:demoIDs[0],autoplay:saved.compo?saved.compo.autoplay!==false:true,elapsed:0};
let compoSession=!!saved.compoSession||compo.enabled, compoLast=nativeTime();
function selectDemo(id,autoplay,remember=true){
 if(!demoIDs.includes(id))throw Error('unknown demo');
 compo={enabled:true,id,autoplay:!!autoplay,elapsed:0,paused:false};compoSession=true;clock=false;clockApplied=false;compoLast=nativeTime();if(remember)persist();
}
const digits=['111101101101111','010110010010111','111001111100111','111001111001111','101101111001001','111100111001111','111100111101111','111001010010010','111101111101111','111101111001111'];
function persist(){if(!nativeSave(JSON.stringify({version:1,scene:selected,clock,brightness,day,count,revision,last_sync:lastSync,theme,compo:{enabled:compo.enabled,id:compo.id,autoplay:compo.autoplay},compoSession})))throw Error('state persistence failed');}
function api(method,path,value){return nativeApi(method,path,JSON.stringify(value));}
function setScene(id,keepCompo=false){if(!ids.includes(id))throw Error('unknown scene');selected=id;compo.enabled=false;if(!keepCompo)compoSession=false;clock=false;returnClock=false;clockApplied=false;if(active){active.start=nativeTime();active.duration=art.themes.find(x=>x.id===selected).duration;}persist();}
function setClock(value){compo.enabled=false;compoSession=false;clock=value;returnClock=false;clockApplied=false;persist();if(clock&&!active){api('PUT','/scene',{base:'clock'});clockApplied=true;}}
function resetDay(){if(today()===day)return;day=today();count=0;revision=0;visible=0;queue=[];active=null;returnClock=false;persist();}
function finish(){if(!active)return;if(!active.demo)visible=active.n;active=null;if(!queue.length&&returnClock){returnClock=false;clock=true;clockApplied=false;}}
function begin(){if(active||!queue.length)return;let event=queue.shift();active={...event,start:nativeTime(),duration:art.themes.find(x=>x.id===selected).duration};clockApplied=false;}
function replay(){if(active)return;queue.unshift({n:Math.max(1,count),demo:true});begin();}
g.storyControl=function(control,event){
 if(event==='long')return; // hold encoder retains the native settings menu
 if(event==='cw'||event==='ccw'){
  brightness=Math.max(5,Math.min(75,brightness+(event==='cw'?5:-5)));
  api('POST','/action',{action:'brightness',brightness});persist();return;
 }
 if(event!=='click')return;
 if(compo.enabled&&(control==='left'||control==='right')){selectDemo(demoIDs[(demoIDs.indexOf(compo.id)+(control==='right'?1:demoIDs.length-1))%demoIDs.length],false);return;}
 if(compo.enabled&&control==='knob'){compo.autoplay=!compo.autoplay;persist();return;}
 if(control==='middle'&&compoSession){
  if(compo.enabled){compo.enabled=false;clock=false;}
  else if(!clock)clock=true;
  else{clock=false;compo.enabled=true;compo.elapsed=0;compoLast=nativeTime();}
  clockApplied=false;persist();return;
 }
 if(control==='left'||control==='right')setScene(ids[(ids.indexOf(selected)+(control==='right'?1:ids.length-1))%ids.length],compoSession);
 else if(control==='middle')setClock(!clock);
 else if(control==='knob')replay();
};
function status(){return {version:1,engine:'local-quickjs',palette_rendering:'contrast-accents-v1',resolution:[52,16],scene:selected,scenes:ids,day,count,visual_capacity:20,visible_count:visible,revision,clock,brightness,last_sync:lastSync,queue:queue.length,animation:active?{ordinal:active.n,demo:active.demo,seconds:Math.max(0,nativeTime()-active.start),duration:active.duration}:null,theme,mode:compo.enabled?'compo':clock?'clock':'stories',compo:demos?{...compo,number:demoIDs.indexOf(compo.id)+1,name:demos.catalogue.find(s=>s.id===compo.id).name,total:demoIDs.length}:null,...nativeInfo()};}
g.storyRequest=function(method,path,body){
 resetDay();
 if(method==='GET'&&path==='/status')return status();
 if(method==='GET'&&path==='/demos')return demos?demos.catalogue:[];
 if(method==='POST'&&path==='/demo'){
  if(!demos)throw Error('compo module not installed');
  if(body.enabled===false){compo.enabled=false;compoSession=false;clock=false;persist();return status();}
  if(body.autoplay!==undefined&&typeof body.autoplay!=='boolean')throw Error('autoplay must be boolean');
  if(body.time!==undefined&&(!Number.isFinite(body.time)||body.time<0||body.time>3600))throw Error('invalid demo time');
  if(body.paused!==undefined&&typeof body.paused!=='boolean')throw Error('invalid pause state');
  selectDemo(body.id||compo.id,body.autoplay!==false);
  if(body.time!==undefined)compo.elapsed=1.15+body.time;
  compo.paused=body.paused===true;return status();
 }
 if(method==='PUT'&&path==='/state'){
  if(body.day!==today()||!Number.isSafeInteger(body.count)||body.count<0||body.count>100000||!Number.isSafeInteger(body.revision)||body.revision<1)throw Error('invalid daily state');
  lastSync=Number.isFinite(body.asof)?body.asof:lastSync;
  if(body.revision<=revision)return {...status(),duplicate:true};
  const old=count;count=body.count;revision=body.revision;
  if(body.animate===true&&count>old){
   if(clock)returnClock=true;
   for(let n=old+1;n<=count;n++)queue.push({n,demo:false});
   // Avoid an unbounded backlog after a long outage, while keeping the exact count.
   if(queue.length>100)queue=queue.slice(-100);
  }else{queue=[];active=null;visible=count;}
  persist();begin();return status();
 }
 if(method==='POST'&&path==='/clock'){setClock(true);return status();}
 if(method==='POST'&&path==='/scene'){setScene(body.scene);return status();}
 if(method==='POST'&&path==='/input'){
  if(!['left','middle','right','knob','rotary'].includes(body.control)||!['click','long','cw','ccw'].includes(body.event))throw Error('invalid input');
  g.storyControl(body.control,body.event);return status();
 }
 if(method==='POST'&&path==='/replay'){replay();return status();}
 if(method==='PUT'&&path==='/palette'){
  if(!/^[0-9a-fA-F]{6}$/.test(body.accent)||!/^[0-9a-fA-F]{6}$/.test(body.secondary)||typeof body.name!=='string'||body.name.length>100)throw Error('invalid palette');
  const roles=['foreground','background','selection','red','green','blue','yellow','orange','cyan','magenta','brown'];
  const next={name:body.name,accent:body.accent,secondary:body.secondary};
  if(body.colors!==undefined){
   if(!body.colors||typeof body.colors!=='object'||Array.isArray(body.colors))throw Error('invalid theme colours');
   if(Object.keys(body.colors).some(k=>!roles.includes(k)))throw Error('unknown theme colour');
   next.colors={};for(const key of roles)if(body.colors[key]!==undefined){if(!/^[0-9a-fA-F]{6}$/.test(body.colors[key]))throw Error('invalid theme colour');next.colors[key]=body.colors[key];}
  }
  const key=value=>[value.name,value.accent,value.secondary,...roles.map(k=>(value.colors||{})[k]||'')].join('/');
  if(key(theme)!==key(next)){theme=next;maps=Object.fromEntries(ids.map(id=>[id,paletteMap(id)]));persist();}return status();
 }
 throw Error('unknown request');
};
function number(data,n,colour){const text=String(n);let width=text.length*4-1;for(let y=0;y<5;y++)for(let x=0;x<width;x++)data[y*52+x]=0;
 [...text].forEach((c,i)=>[...digits[+c]].forEach((b,j)=>{if(b==='1')data[Math.floor(j/3)*52+i*4+j%3]=colour;}));}
g.storyTick=function(){
 const now=nativeTime(),delta=Math.max(0,Math.min(1,now-compoLast));compoLast=now;
 resetDay();if(active&&nativeTime()-active.start>=active.duration)finish();begin();
 if(clock&&!active){if(!clockApplied){api('PUT','/scene',{base:'clock'});clockApplied=true;}return null;}
 clockApplied=false;
 if(compo.enabled&&!active){
  if(!compo.paused)compo.elapsed+=delta;const meta=demos.catalogue.find(s=>s.id===compo.id);
  if(compo.autoplay&&compo.elapsed>=meta.duration+1.15){selectDemo(demoIDs[(demoIDs.indexOf(compo.id)+1)%demoIDs.length],true,false);}
  const elapsed=compo.autoplay||compo.elapsed<1.15?compo.elapsed:1.15+(compo.elapsed-1.15)%meta.duration;
  return demos.rgb(demos.frame(compo.id,elapsed,theme,{title:true}));
 }
 let n=active?active.n:visible,e=active?Math.max(0,(nativeTime()-active.start)/active.duration):null;
 const overflow=n>20;let data=art.frame(selected,Math.min(20,n),nativeTime(),overflow?null:e,{counter:false});
 // A completed 20-object world stays intact beyond 20; later payments still celebrate.
 if(overflow&&active){const s=nativeTime()-active.start;if(s<5)for(let i=0;i<8;i++){let x=(i*7+Math.floor(s*3))%52,y=Math.floor(s*2+i*3)%16;data[y*52+x]=[art.C.gold,art.C.cyan,art.C.pink][i%3];}}
 applyPalette(data,selected);
 number(data,count,theme.accent?ledColour(theme.accent,255):art.C.amber);return art.rgb(data);
};
})(globalThis);
