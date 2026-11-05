/* Runs identically in native QuickJS and the deterministic Node tests. */
(function(g){
'use strict';
const art=g.UlanziOrders, ids=art.scenes.map(x=>x.id), today=()=>nativeDay(), MAX_ORDERS=100;
// Local wall clock (storyd sets TZ to Europe/Warsaw); scenes place orders and "now" on the day.
const secondsToday=()=>{const d=new Date();return d.getHours()*3600+d.getMinutes()*60+d.getSeconds()+d.getMilliseconds()/1000;};
let saved=nativeLoad()||{}, day=today(), count=0, revision=0, orders=[], selected=ids.includes(saved.scene)?saved.scene:ids[0];
let clock=!!saved.clock, brightness=Number.isInteger(saved.brightness)?saved.brightness:35;
if(saved.day===day){count=Number.isInteger(saved.count)?Math.max(0,saved.count):0;revision=saved.revision||0;orders=Array.isArray(saved.orders)?saved.orders.slice(-MAX_ORDERS):[];}
// Arrival sounds: on unless muted (hold Ulanzi), silent at night except for a replay asked for by hand.
const QUIET_FROM=22,QUIET_TO=7,SOUND_VOLUME=60;
let sound=saved.sound!==false,toastUntil=0;
const quiet=()=>{const h=secondsToday()/3600;return h>=QUIET_FROM||h<QUIET_TO;};
let lastSync=saved.last_sync||0, queue=[],active=null, returnClock=false, clockApplied=false, theme=saved.theme||{};
const demos=g.UlanziDemos||null, demoIDs=demos?demos.catalogue.map(s=>s.id):[];
let compo={enabled:!!(saved.compo&&saved.compo.enabled&&demos),id:saved.compo&&demoIDs.includes(saved.compo.id)?saved.compo.id:demoIDs[0],autoplay:saved.compo?saved.compo.autoplay!==false:true,elapsed:0};
let compoSession=!!saved.compoSession||compo.enabled, compoLast=nativeTime();
function selectDemo(id,autoplay,remember=true){
 if(!demoIDs.includes(id))throw Error('unknown demo');
 compo={enabled:true,id,autoplay:!!autoplay,elapsed:0,paused:false};compoSession=true;clock=false;clockApplied=false;compoLast=nativeTime();if(remember)persist();
}
const duration=()=>art.scenes.find(x=>x.id===selected).duration;
function persist(){if(!nativeSave(JSON.stringify({version:2,scene:selected,clock,brightness,sound,day,count,revision,orders,last_sync:lastSync,theme,compo:{enabled:compo.enabled,id:compo.id,autoplay:compo.autoplay},compoSession})))throw Error('state persistence failed');}
function api(method,path,value){return nativeApi(method,path,JSON.stringify(value));}
function setScene(id,keepCompo=false){if(!ids.includes(id))throw Error('unknown scene');selected=id;compo.enabled=false;if(!keepCompo)compoSession=false;clock=false;returnClock=false;clockApplied=false;if(active){active.start=nativeTime();active.duration=duration();active.cue=null;}persist();}
function setClock(value){compo.enabled=false;compoSession=false;clock=value;returnClock=false;clockApplied=false;persist();if(clock&&!active){api('PUT','/scene',{base:'clock'});clockApplied=true;}}
function resetDay(){if(today()===day)return;day=today();count=0;revision=0;orders=[];queue=[];active=null;returnClock=false;persist();}
function finish(){if(!active)return;active=null;if(!queue.length&&returnClock){returnClock=false;clock=true;clockApplied=false;}}
function begin(){if(active||!queue.length)return;active={...queue.shift(),start:nativeTime(),duration:duration()};clockApplied=false;}
function replay(){if(active||!count)return;queue.unshift({n:count,demo:true});begin();}
g.storyControl=function(control,event){
 if(event==='long'&&control==='middle'){sound=!sound;toastUntil=nativeTime()+1.6;persist();return;}
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
function status(){return {version:2,engine:'local-quickjs',resolution:[52,16],scene:selected,scenes:ids,day,count,orders_known:orders.length,revision,clock,brightness,sound:{enabled:sound,quiet_hours:QUIET_FROM+'-'+QUIET_TO,available:typeof nativeSound==='function'},last_sync:lastSync,queue:queue.length,animation:active?{ordinal:active.n,demo:active.demo,seconds:Math.max(0,nativeTime()-active.start),duration:active.duration}:null,theme,mode:compo.enabled?'compo':clock?'clock':'stories',compo:demos?{...compo,number:demoIDs.indexOf(compo.id)+1,name:demos.catalogue.find(s=>s.id===compo.id).name,total:demoIDs.length}:null,...nativeInfo()};}
// Orders arrive as [seconds since local midnight, value in whole złoty or null], oldest first.
function validOrders(list,n){
 return Array.isArray(list)&&list.length===Math.min(n,MAX_ORDERS)&&list.every((o,i)=>Array.isArray(o)&&o.length===2&&Number.isInteger(o[0])&&o[0]>=0&&o[0]<90000&&(o[1]===null||Number.isInteger(o[1])&&o[1]>=0&&o[1]<=10000000)&&(!i||o[0]>=list[i-1][0]));
}
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
  if(body.orders!==undefined&&!validOrders(body.orders,body.count))throw Error('invalid orders');
  lastSync=Number.isFinite(body.asof)?body.asof:lastSync;
  if(body.revision<=revision)return {...status(),duplicate:true};
  const old=count;count=body.count;revision=body.revision;
  // A bridge that sends only the count: new orders are placed at the moment they reached the clock.
  if(body.orders!==undefined)orders=body.orders;
  else{const now=Math.floor(secondsToday());orders=orders.slice(0,count);while(orders.length<Math.min(count,MAX_ORDERS))orders.push([now,null]);}
  if(body.animate===true&&count>old){
   if(clock)returnClock=true;
   for(let n=old+1;n<=count;n++)queue.push({n,demo:false});
   // Avoid an unbounded backlog after a long outage, while keeping the exact count.
   if(queue.length>100)queue=queue.slice(-100);
  }else{queue=[];active=null;}
  persist();begin();return status();
 }
 if(method==='PUT'&&path==='/sound'){if(typeof body.enabled!=='boolean')throw Error('enabled must be boolean');if(body.enabled!==sound){sound=body.enabled;persist();}return status();}
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
  if(key(theme)!==key(next)){theme=next;persist();}return status();
 }
 throw Error('unknown request');
};
// What a scene shows: during an arrival, orders after the arriving one are still hidden.
function view(){
 const n=active?active.n:count,hidden=count-n,list=orders.slice(0,Math.max(0,orders.length-hidden)).map(([t,v])=>({t,v}));
 return {orders:list,count:n,now:secondsToday(),event:active?{s:Math.max(0,nativeTime()-active.start)}:null};
}
g.storyTick=function(){
 const now=nativeTime(),delta=Math.max(0,Math.min(1,now-compoLast));compoLast=now;
 resetDay();if(active&&nativeTime()-active.start>=active.duration)finish();begin();
 if(clock&&!active){if(!clockApplied){api('PUT','/scene',{base:'clock'});clockApplied=true;}return null;}
 clockApplied=false;
 if(compo.enabled&&!active){
  if(!compo.paused)compo.elapsed+=delta;const meta=demos.catalogue.find(s=>s.id===compo.id);
  if(compo.autoplay&&compo.elapsed>=meta.duration+1.15){selectDemo(demoIDs[(demoIDs.indexOf(compo.id)+1)%demoIDs.length],true,false);}
  const elapsed=compo.autoplay||compo.elapsed<1.15?compo.elapsed:1.15+(compo.elapsed-1.15)%meta.duration;
  const data=demos.frame(compo.id,elapsed,theme,{title:true});if(now<toastUntil)speaker(data);return demos.rgb(data);
 }
 const v=view();
 // The arrival sound starts at its scene's moment: the bead lands, the comet sets off, the marble touches.
 if(active&&!active.played){active.cue=active.cue||art.cue(selected,v);
  if(v.event.s>=active.cue.at){active.played=true;if(sound&&(active.demo||!quiet())&&typeof nativeSound==='function')nativeSound(active.cue.name,SOUND_VOLUME);}}
 const data=art.frame(selected,v,theme,now);if(now<toastUntil)speaker(data);
 return art.rgb(data);
};
// A small speaker in the top-left corner after the mute toggle: waves when on, a cross when off.
function speaker(data){
 const rows=sound?['...#.....','..##..#..','####.#.#.','####.#.#.','..##..#..','...#.....']:['...#.....','..##.#.#.','####..#..','####.#.#.','..##.....','...#.....'];
 for(let y=0;y<8;y++)for(let x=0;x<11;x++)data[y*52+x]=0;
 rows.forEach((r,y)=>[...r].forEach((b,x)=>{if(b==='#')data[(y+1)*52+x+1]=0xffffff;}));
}
})(globalThis);
