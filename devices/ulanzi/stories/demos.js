/* Compo: eight seamless loops at native 52×16. They draw with the order scenes' rasterizer and theme palette
 * (renderer.js, loaded first by storyd), so the whole clock speaks one visual language. */
(function(g){
'use strict';
const P=g.UlanziPixels||(typeof require==='function'?(require('./renderer.js'),g.UlanziPixels):null);
const {W,H,surface,scale,blend,hsv,clamp,ease,part,unit,frac}=P,TAU=Math.PI*2;
const catalogue=[
 ['pendulums','Pendulums','Trzynaście wahadeł o coraz krótszym okresie: fale, chaos i powrót do jednej linii',30],
 ['fireflies','Fireflies','Świetliki nad trawą powoli zgrywają błyski, a potem znów się rozchodzą',24],
 ['harmonograph','Harmonograph','Wygasające wahadła rysują figurę, która obraca się i gaśnie',24],
 ['ripples','Ripples','Krople deszczu i kręgi rozchodzące się po ciemnej wodzie',20],
 ['aurora','Aurora','Zasłony zorzy falują nad pasmem gór',24],
 ['bloom','Bloom','Trzy kwiaty otwierają się i zamykają w jednym oddechu',20],
 ['lace','Lace','Koronka z łuków: kafelki obracają się jeden po drugim, a wzór płynie dalej',24],
 ['tide','Tide','Księżyc, srebrna ścieżka na falach i chmura, która go mija',24]
].map((a,i)=>({id:a[0],name:a[1],description:a[2],duration:a[3],number:i+1}));
const FONT={A:'010101111101101',B:'110101110101110',C:'011100100100011',D:'110101101101110',E:'111100110100111',F:'111100110100100',G:'011100101101011',H:'101101111101101',I:'111010010010111',J:'001001001101010',K:'101101110101101',L:'100100100100111',M:'101111111101101',N:'101111111111101',O:'010101101101010',P:'110101110100100',Q:'010101101111011',R:'110101110101101',S:'011100010001110',T:'111010010010010',U:'101101101101111',V:'101101101101010',W:'101101111111101',X:'101101010101101',Y:'101101010010010',Z:'111001010100111',' ':'000000000000000'};
// Each loop's own palette roles: two theme hues, the warmest theme hue for light sources, and the foreground.
let roleKey=null,roleCache=null;
function roles(theme){
 const p=P.palette(theme);if(p===roleKey)return roleCache;
 const warm=[...p.hues].sort((a,b)=>Math.abs(hsv(a).h-48)-Math.abs(hsv(b).h-48))[0];
 roleKey=p;roleCache={a:p.hues[0],b:p.hues[1],c:p.hues[2],hues:p.hues,warm,white:p.white};return roleCache;
}
const scenes={};

scenes.pendulums=(d,t,c)=>{
 // Pendulum i makes 18+i swings per loop, so all thirteen line up again exactly when the loop ends.
 for(let i=0;i<13;i++){
  const x=2+i*4,f=(18+i)/30,col=blend(c.a,c.b,i/12),y=k=>7.5+6*Math.cos(TAU*f*(t-k*.03));
  d.line(x,0,x,y(0)-1.3,scale(col,.045));
  for(let k=3;k>0;k--){const r=1.1-k*.12;d.ellipse(x,y(k),r,r,scale(col,.42-.11*k),true);}
  d.ellipse(x,y(0),1.15,1.15,col,true);
 }
};

scenes.fireflies=(d,t,c)=>{
 const w=TAU/24,together=part(t,3,12)*(1-part(t,16,22));
 for(let x=0;x<W;x+=2){const h=1.5+2.6*unit(x*7+1),sway=.6*Math.sin(w*3*t+x*.37);d.line(x,15.5,x+sway,15.5-h,scale(c.a,.07+.04*unit(x)));}
 for(let i=0;i<14;i++){
  // Flashes every two seconds; their phases drift together, hold, and drift apart again.
  const phase=frac(t/2+unit(i*19+5)*(1-together)),b=Math.max(0,1-phase/.4)**2;
  const x=3+46*unit(i*3+1)+3*Math.sin(w*(1+i%3)*t+i),y=2.5+8*unit(i*5+2)+1.6*Math.sin(w*(2+i%2)*t+i*2);
  if(b<.02)continue;
  d.ellipse(x,y,2,2,scale(c.warm,.22*b),true);d.ellipse(x,y,1.1,1.1,scale(c.warm,.65*b),true);d.ellipse(x,y,.55,.55,blend(c.warm,c.white,.45*b),true);
 }
};

scenes.harmonograph=(d,t,c)=>{
 const fade=1-part(t,20.5,23.5),pen=u=>{const a=Math.exp(-.026*u);return [26+a*21*Math.sin(3*u+u*.006),7.5+a*6.6*Math.sin(2*u+.6)];};
 const head=t*2.4,steps=110,step=.082;
 let prev=null;
 for(let k=steps;k>=0;k--){
  const u=head-k*step;if(u<0)continue;const [x,y]=pen(u),age=k/steps;
  if(prev){const col=age<.5?blend(c.white,c.a,age*2):blend(c.a,c.b,age*2-1);d.line(prev[0],prev[1],x,y,scale(col,fade*(1-age)**1.2));}
  prev=[x,y];
 }
 if(prev)d.ellipse(prev[0],prev[1],.7,.7,scale(c.white,fade),true);
};

scenes.ripples=(d,t,c)=>{
 const rings=[];
 for(let k=0;k<15;k++){
  const at=k*1.3+.45*unit(k*29),x=5+42*unit(k*11+3),y=6.5+7.5*unit(k*13+7),col=c.hues[k%3];
  for(const wrap of [0,20]){const age=t+wrap-at;
   if(age>-.35&&age<0){const q=1+age/.35;d.rect(x-.5,y-6+6*q*q-.5,1,1.4,scale(c.white,.7));}
   if(age>=0&&age<3.4)rings.push({x,y,age,col});}
 }
 rings.sort((a,b)=>b.age-a.age).forEach(({x,y,age,col})=>{
  const fade=(1-age/3.4)**1.6;
  // Rings are traced as points, so they stay one pixel thin however wide they get.
  for(const [lag,q] of [[0,1],[.55,.5]]){if(age<lag)continue;const r=.6+5.6*(age-lag)**.75,ink=scale(age<.15&&!lag?blend(col,c.white,.6):col,fade*q),n=Math.max(8,Math.round(r*5));
   for(let k=0;k<n;k++){const a=k*TAU/n;d.rect(x+r*Math.cos(a)-.5,y+r*.38*Math.sin(a)-.5,1,1,ink);}}
 });
};

const RIDGE=Array.from({length:W},(_,x)=>12.6+1.5*Math.sin(x*.19)+1.1*Math.sin(x*.47+1)+.5*Math.sin(x*1.3));
scenes.aurora=(d,t,c)=>{
 // Whole pixels only: written straight into the frame, like the stained glass.
 const w=TAU/24,out=new Uint32Array(W*H),put=(x,y,col,q)=>{out[y*W+x]=((col>>>16&255)*q<<16)|((col>>>8&255)*q<<8)|((col&255)*q);};
 for(let i=0;i<9;i++)put(Math.floor(52*unit(i*17+3)),Math.floor(9*unit(i*23+1)),c.white,.18+.1*Math.sin(w*(3+i%4)*t+i));
 const ramp=Array.from({length:H},(_,k)=>blend(c.b,c.a,Math.min(1,k/12)));
 for(let x=0;x<W;x++){
  const top=2.4+1.9*Math.sin(x*.13+w*2*t)+1.1*Math.sin(x*.31-w*3*t),len=5+3*Math.sin(x*.07-w*t+1);
  const glow=clamp(.5+.38*Math.sin(x*.22-w*4*t+1.6*Math.sin(x*.05+w*2*t))+.22*Math.sin(x*1.1+w*6*t+Math.sin(x*.3)));
  const ridge=Math.round(RIDGE[x]);
  for(let y=Math.max(0,Math.ceil(top));y<Math.min(ridge,top+len);y++){
   const q=(y-top)/len,b=glow*(q<.18?q/.18:(1-q)**1.4);if(b>.04)put(x,y,ramp[Math.min(H-1,Math.round(q*16))],b);
  }
  for(let y=ridge;y<H;y++)out[y*W+x]=0;
  put(x,ridge-1>=0?ridge-1:0,c.a,.1+.18*glow);
 }
 return out;
};

function flower(d,cx,cy,R,open,turn,c){
 const layer=(n,rr,col,offset)=>{for(let i=0;i<n;i++){const a=turn+offset+i*TAU/n,ext=rr*open;
  [[.3,.12,.7],[.55,.17,1],[.8,.13,.85],[.97,.07,.7]].forEach(([f,r,q])=>{const x=cx+Math.cos(a)*ext*f,y=cy+Math.sin(a)*ext*f*.82;d.ellipse(x,y,R*r,R*r*.9,scale(col,q),true);});}};
 layer(6,R,c.b,0);layer(6,R*.62,c.a,TAU/12);
 d.ellipse(cx,cy,R*.17+.3,R*.15+.3,c.warm,true);
}
scenes.bloom=(d,t,c)=>{
 const breath=s=>{s=((s%20)+20)%20;return .3+.7*ease(part(s,1,7))*(1-part(s,12.5,18.5));};
 flower(d,9,9.5,3.8,breath(t-2.2),TAU*t/120+.3,c);
 flower(d,43,9.5,3.8,breath(t-1.1),-TAU*t/120+.1,c);
 const open=breath(t);flower(d,26,7.6,6.6,open,TAU*t/120,c);
 if(open>.8)for(let i=0;i<6;i++){const q=frac(t/3+i/6),x=26+Math.sin(i*2.1+q*4)*3*q,y=7.6-q*7;d.dot(x,y,scale(c.warm,(open-.8)*5*(1-q)));}
};

// Truchet lace: every tile flips twice per loop, so the pattern returns to where it began.
const TILES=Array.from({length:12},(_,i)=>({x:i%6,y:Math.floor(i/6),first:24*unit(i*31+9),start:unit(i*7+2)<.5?0:1}));
scenes.lace=(d,t,c)=>{
 const w=TAU/24;
 for(const tile of TILES){
  const cx=2+tile.x*8+3.5,cy=tile.y*8+3.5;let o=tile.start,turn=0;
  for(const at of [tile.first,(tile.first+12)%24]){if(t>=at+.7)o^=1;else if(t>=at)turn=ease((t-at)/.7);}
  const col=blend(c.a,c.b,tile.x/5),shine=.5+.5*Math.max(0,Math.cos(cx*.13-w*3*t)),ink=scale(blend(col,c.white,turn>0&&turn<1?.5:0),.45+.55*shine);
  const rot=(o+turn)*Math.PI/2,cs=Math.cos(rot),sn=Math.sin(rot);
  for(const [kx,ky,a0] of [[-4,-4,0],[4,4,Math.PI]]){
   let prev=null;
   for(let k=0;k<=8;k++){const a=a0+k*Math.PI/16,px=kx+4*Math.cos(a),py=ky+4*Math.sin(a),x=cx+px*cs-py*sn,y=cy+px*sn+py*cs;if(prev)d.line(prev[0],prev[1],x,y,ink);prev=[x,y];}
  }
 }
};

scenes.tide=(d,t,c)=>{
 const w=TAU/24,mx=39,my=3.6;
 for(let i=0;i<8;i++){const x=Math.floor(30*unit(i*13+2)),y=Math.floor(7*unit(i*5+9));d.dot(x,y,scale(c.white,.14+.08*Math.sin(w*(2+i%3)*t+i)));}
 d.ellipse(mx,my,2.3,2.3,scale(c.white,.95),true);d.ellipse(mx-.6,my-.5,.6,.6,scale(c.white,.75),true);
 // A moonlit cloud crosses once per loop and veils the moon on its way.
 const cx=-14+80*t/24;[[0,0,5,1.3],[3.5,-.8,3.4,1.2],[-3.4,.3,3,1]].forEach(([dx,dy,rx,ry])=>d.ellipse(cx+dx,my+.6+dy,rx,ry,scale(c.b,.09),true));
 d.line(0,8,51,8,scale(c.a,.05));
 for(let y=9;y<H;y++)for(let x=0;x<W;x++){
  const near=Math.exp(-(((x-mx)/(1.6+(y-8)*1.1))**2)),wave=Math.sin(x*.9+y*1.7+w*5*t)*Math.sin(x*.37-y+w*2*t+Math.sin(y*.8+w*t));
  const b=wave>.5-.45*near?(.1+.9*near)*(wave+.2-.2*near)*.9:0;if(b>.05)d.dot(x,y,blend(scale(c.a,b),scale(c.white,b),near));
 }
};

function text(d,str,x,y,col){[...str.toUpperCase()].forEach((k,i)=>{const bits=FONT[k]||FONT[' '];for(let j=0;j<15;j++)if(bits[j]==='1')d.dot(x+i*4+j%3,y+Math.floor(j/3),col);});}
function frame(id,t=0,theme={},options={}){
 const meta=catalogue.find(s=>s.id===id);if(!meta)throw Error('Unknown demo '+id);
 const d=surface(),c=roles(theme);t=Math.max(0,t);
 if(options.title&&t<1.15){const name=meta.name.toUpperCase(),a=part(t,0,.2)*(1-part(t,.85,1.15));text(d,name,Math.round((W-(name.length*4-1))/2),5,scale(c.a,a));}
 else{const direct=scenes[id](d,Math.round(((options.title?t-1.15:t)%meta.duration)*1e6)/1e6,c);if(direct)return direct;}
 return d.finish();
}
g.UlanziDemos={W,H,catalogue,frame,rgb:P.rgb,palette:roles};
if(typeof module!=='undefined'&&module.exports)module.exports=g.UlanziDemos;
})(typeof globalThis!=='undefined'?globalThis:this);
