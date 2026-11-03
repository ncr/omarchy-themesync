/* Runs identically in native QuickJS and the deterministic Node tests. */
(function(g){
'use strict';
const art=g.UlanziConcepts, ids=art.themes.map(x=>x.id), today=()=>nativeDay();
let saved=nativeLoad()||{}, day=today(), count=0, revision=0, selected=ids.includes(saved.scene)?saved.scene:'rail';
let clock=!!saved.clock, brightness=Number.isInteger(saved.brightness)?saved.brightness:35;
if(saved.day===day){count=Number.isInteger(saved.count)?Math.max(0,saved.count):0;revision=saved.revision||0;}
let lastSync=saved.last_sync||0, queue=[],active=null, visible=count, returnClock=false, clockApplied=false, theme=saved.theme||{};
const digits=['111101101101111','010110010010111','111001111100111','111001111001111','101101111001001','111100111001111','111100111101111','111001010010010','111101111101111','111101111001111'];
function persist(){if(!nativeSave(JSON.stringify({version:1,scene:selected,clock,brightness,day,count,revision,last_sync:lastSync,theme})))throw Error('state persistence failed');}
function api(method,path,value){return nativeApi(method,path,JSON.stringify(value));}
function setScene(id){if(!ids.includes(id))throw Error('unknown scene');selected=id;clock=false;returnClock=false;clockApplied=false;if(active){active.start=nativeTime();active.duration=art.themes.find(x=>x.id===selected).duration;}persist();}
function setClock(value){clock=value;returnClock=false;clockApplied=false;persist();if(clock&&!active){api('PUT','/scene',{base:'clock'});clockApplied=true;}}
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
 if(control==='left'||control==='right')setScene(ids[(ids.indexOf(selected)+(control==='right'?1:ids.length-1))%ids.length]);
 else if(control==='middle')setClock(!clock);
 else if(control==='knob')replay();
};
function status(){return {version:1,engine:'local-quickjs',resolution:[52,16],scene:selected,scenes:ids,day,count,visual_capacity:20,visible_count:visible,revision,clock,brightness,last_sync:lastSync,queue:queue.length,animation:active?{ordinal:active.n,demo:active.demo,seconds:Math.max(0,nativeTime()-active.start),duration:active.duration}:null,theme,...nativeInfo()};}
g.storyRequest=function(method,path,body){
 resetDay();
 if(method==='GET'&&path==='/status')return status();
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
  if(theme.name!==body.name||theme.accent!==body.accent||theme.secondary!==body.secondary){theme={name:body.name,accent:body.accent,secondary:body.secondary};persist();}return status();
 }
 throw Error('unknown request');
};
function number(data,n,colour){const text=String(n);let width=text.length*4-1;for(let y=0;y<5;y++)for(let x=0;x<width;x++)data[y*52+x]=0;
 [...text].forEach((c,i)=>[...digits[+c]].forEach((b,j)=>{if(b==='1')data[Math.floor(j/3)*52+i*4+j%3]=colour;}));}
g.storyTick=function(){
 resetDay();if(active&&nativeTime()-active.start>=active.duration)finish();begin();
 if(clock&&!active){if(!clockApplied){api('PUT','/scene',{base:'clock'});clockApplied=true;}return null;}
 clockApplied=false;
 let n=active?active.n:visible,e=active?Math.max(0,(nativeTime()-active.start)/active.duration):null;
 const overflow=n>20;let data=art.frame(selected,Math.min(20,n),nativeTime(),overflow?null:e,{counter:false});
 // A completed 20-object world stays intact beyond 20; later payments still celebrate.
 if(overflow&&active){const s=nativeTime()-active.start;if(s<5)for(let i=0;i<8;i++){let x=(i*7+Math.floor(s*3))%52,y=Math.floor(s*2+i*3)%16;data[y*52+x]=[art.C.gold,art.C.cyan,art.C.pink][i%3];}}
 number(data,count,art.C.amber);return art.rgb(data);
};
})(globalThis);
