/* Today's paid orders as five native 52×16 scenes. Shared by QuickJS on the TC002, the preview and the tests.
 * A scene sees only when each order was paid and its value, never who bought what. */
(function(g){
'use strict';
const W=52,H=16,DAY=86400,TAU=Math.PI*2;
const scenes=[
 {id:'hours',name:'Liczydło',description:'Każda godzina doby to pręt, każde zamówienie to koralik w kolorze swojej wartości',duration:5},
 {id:'sky',name:'Gwiazdozbiór',description:'Zamówienia to gwiazdy: w poziomie godzina, w pionie wartość, linie łączą je po kolei',duration:6},
 {id:'jar',name:'Słoik',description:'Kulki spadają i układają się w kopiec; rozmiar to wartość, kolor to pora dnia',duration:6},
 {id:'board',name:'Tablica',description:'Tablica przylotów: liczba zamówień i godziny dwóch ostatnich',duration:5},
 {id:'glass',name:'Witraż',description:'Każde zamówienie to szybka witraża, większa przy większej wartości',duration:6}
];
const clamp=(v,a=0,b=1)=>v<a?a:v>b?b:v,mix=(a,b,t)=>a+(b-a)*clamp(t),ease=t=>{t=clamp(t);return t*t*(3-2*t);},part=(t,a,b)=>ease((t-a)/(b-a));
const frac=x=>x-Math.floor(x);
function hash(n){n=Math.imul(n^0x9e3779b9,0x85ebca6b);n=Math.imul(n^(n>>>13),0xc2b2ae35);return (n^(n>>>16))>>>0;}
const unit=n=>hash(n)/4294967296;

/* Colour. Theme roles are pushed to full LED brightness; scenes pick hues that stay apart. */
const R=c=>c>>>16&255,G=c=>c>>>8&255,B=c=>c&255,byte=v=>v<0?0:v>255?255:Math.round(v);
const rgb3=(r,gr,b)=>(byte(r)<<16)|(byte(gr)<<8)|byte(b);
const scale=(c,q)=>rgb3(R(c)*q,G(c)*q,B(c)*q);
const blend=(a,b,t)=>{t=clamp(t);return rgb3(R(a)+(R(b)-R(a))*t,G(a)+(G(b)-G(a))*t,B(a)+(B(b)-B(a))*t);};
const parse=hex=>typeof hex==='string'&&/^#?[0-9a-fA-F]{6}$/.test(hex)?parseInt(hex.replace('#',''),16):null;
function vivid(c){const r=R(c),gr=G(c),b=B(c),lo=Math.min(r,gr,b),k=Math.max(r,gr,b)>lo?.45:0,hi=Math.max(r,gr,b)-lo*k;return hi<8?0:rgb3((r-lo*k)*255/hi,(gr-lo*k)*255/hi,(b-lo*k)*255/hi);}
function bright(c){const hi=Math.max(R(c),G(c),B(c));return hi<8?0xffffff:scale(c,255/hi);}
function hsv(c){const r=R(c),gr=G(c),b=B(c),hi=Math.max(r,gr,b),lo=Math.min(r,gr,b),d=hi-lo;let h=0;
 if(d)h=hi===r?((gr-b)/d+6)%6:hi===gr?(b-r)/d+2:(r-gr)/d+4;return {h:h*60,s:hi?d/hi:0};}
const hueGap=(a,b)=>{const d=Math.abs(hsv(a).h-hsv(b).h)%360;return Math.min(d,360-d);};
function pick(candidates,n){
 // Theme order wins while hues stay clearly apart; only then the most distinct remaining hue.
 const pool=candidates.filter(c=>c!==null&&hsv(c).s>.28),out=pool.slice(0,1);
 for(const c of pool)if(out.length<n&&!out.includes(c)&&Math.min(...out.map(o=>hueGap(c,o)))>=45)out.push(c);
 while(out.length<n){let best=null,gap=-1;for(const c of pool){if(out.includes(c))continue;const d=Math.min(...out.map(o=>hueGap(c,o)));if(d>gap+6){gap=d;best=c;}}if(best===null)break;out.push(best);}
 return out;
}
let paletteKey=null,paletteCache=null;
function palette(theme={}){
 const roles=theme.colors||{},key=JSON.stringify([theme.accent,theme.secondary,roles]);
 if(key===paletteKey)return paletteCache;
 const role=k=>parse(roles[k]),accent=parse(theme.accent)??0x4cc9ff,second=parse(theme.secondary)??role('magenta')??0xb97aff;
 const hues=pick([accent,second,role('yellow'),role('orange'),role('cyan'),role('red'),role('magenta'),role('green'),role('blue'),0xffc400,0xff4a3c,0x38c8ff,0x60e070,0xc060ff],5).map(vivid);
 paletteKey=key;paletteCache={hues,tiers:hues.slice(0,3),white:bright(parse(roles.foreground)??0xe6e6ee)};
 return paletteCache;
}
/* An order's value on a log scale: 5 zł → 0, 50 zł → .5, 500 zł → 1. Unknown values sit in the middle. */
const weight=v=>v===null||v===undefined?.45:clamp((Math.log10(Math.max(1,v))-.699)/2);
const tier=v=>{const w=weight(v);return w<.35?0:w<.7?1:2;};

/* 4×4 coverage samples per physical pixel; the C rasterizer in storyd does the same work natively. */
function surface(){
 const S=4,SW=W*S,SH=H*S,native=typeof nativeRasterBegin==='function';
 if(native)nativeRasterBegin(S);
 const high=native?null:new Uint32Array(SW*SH);
 const span=(a,b,n)=>[Math.max(0,Math.ceil((a+.5)*S-.5)),Math.min(n-1,Math.floor((b+.5)*S-.5))];
 function rect(x,y,w,h,c){
  if(native){nativeRasterRect(x,y,w,h,c);return;}
  if(w<=0||h<=0)return;const [x0,x1]=span(x-.5,x+w-.5,SW),[y0,y1]=span(y-.5,y+h-.5,SH);
  for(let j=y0;j<=y1;j++){const py=(j+.5)/S-.5;if(py<y-.5||py>=y+h-.5)continue;for(let i=x0;i<=x1;i++){const px=(i+.5)/S-.5;if(px>=x-.5&&px<x+w-.5)high[j*SW+i]=c;}}
 }
 function line(x,y,xx,yy,c){
  if(native){nativeRasterLine(x,y,xx,yy,c);return;}
  const [x0,x1]=span(Math.min(x,xx)-.5,Math.max(x,xx)+.5,SW),[y0,y1]=span(Math.min(y,yy)-.5,Math.max(y,yy)+.5,SH),dx=xx-x,dy=yy-y,len=dx*dx+dy*dy;
  for(let j=y0;j<=y1;j++)for(let i=x0;i<=x1;i++){const px=(i+.5)/S-.5,py=(j+.5)/S-.5,q=len?clamp(((px-x)*dx+(py-y)*dy)/len):0,ex=px-x-q*dx,ey=py-y-q*dy;if(ex*ex+ey*ey<=.25)high[j*SW+i]=c;}
 }
 function ellipse(x,y,rx,ry,c,filled=false){
  if(native){nativeRasterEllipse(x,y,rx,ry,c,filled);return;}
  if(rx<=0||ry<=0)return;const [x0,x1]=span(x-rx-.5,x+rx+.5,SW),[y0,y1]=span(y-ry-.5,y+ry+.5,SH),r=Math.min(rx,ry);
  for(let j=y0;j<=y1;j++)for(let i=x0;i<=x1;i++){const dx=((i+.5)/S-.5-x)/rx,dy=((j+.5)/S-.5-y)/ry,v=Math.sqrt(dx*dx+dy*dy);if(filled?v<=1:Math.abs(v-1)*r<=.5)high[j*SW+i]=c;}
 }
 function finish(){
  if(native)return nativeRasterFinish();
  const out=new Uint32Array(W*H),area=S*S;
  for(let y=0;y<H;y++)for(let x=0;x<W;x++){let r=0,gr=0,b=0;for(let j=0;j<S;j++)for(let i=0;i<S;i++){const c=high[(y*S+j)*SW+x*S+i];r+=c>>>16&255;gr+=c>>>8&255;b+=c&255;}out[y*W+x]=(Math.round(r/area)<<16)|(Math.round(gr/area)<<8)|Math.round(b/area);}
  return out;
 }
 return {rect,line,ellipse,finish,dot:(x,y,c)=>rect(x,y,1,1,c)};
}

/* Type: 3×5 for small numbers and times, 5×10 split-flap digits for the board. */
const SMALL={0:'111101101101111',1:'010110010010111',2:'111001111100111',3:'111001111001111',4:'101101111001001',5:'111100111001111',6:'111100111101111',7:'111001010010010',8:'111101111101111',9:'111101111001111','-':'000000111000000',' ':'000000000000000'};
function glyph(d,ch,x,y,c,k=1,cy=y+2.5){const bits=SMALL[ch]||SMALL[' '];if(k<=.02)return;
 for(let j=0;j<5;j++)for(let i=0;i<3;i++)if(bits[j*3+i]==='1')d.rect(x+i,cy+(y+j-cy)*k,1,k,c);}
function number(d,n,x,y,c){[...String(n)].forEach((ch,i)=>glyph(d,ch,x+i*4,y,c));}
const BIG=[
 ['.###.','#...#','#...#','#...#','#...#','#...#','#...#','#...#','#...#','.###.'],
 ['..#..','.##..','#.#..','..#..','..#..','..#..','..#..','..#..','..#..','#####'],
 ['.###.','#...#','....#','....#','...#.','..#..','.#...','#....','#....','#####'],
 ['.###.','#...#','....#','....#','.###.','....#','....#','....#','#...#','.###.'],
 ['...#.','..##.','.#.#.','#..#.','#..#.','#####','...#.','...#.','...#.','...#.'],
 ['#####','#....','#....','#....','####.','....#','....#','....#','#...#','.###.'],
 ['.###.','#...#','#....','#....','####.','#...#','#...#','#...#','#...#','.###.'],
 ['#####','....#','....#','...#.','...#.','..#..','..#..','..#..','..#..','..#..'],
 ['.###.','#...#','#...#','#...#','.###.','#...#','#...#','#...#','#...#','.###.'],
 ['.###.','#...#','#...#','#...#','.####','....#','....#','....#','#...#','.###.']
];
const clock=t=>{const m=Math.floor(t/60)%1440;return String(Math.floor(m/60)).padStart(2,'0')+':'+String(m%60).padStart(2,'0');};

/* Every scene but the board keeps the count in a free corner: top right, else top left, else bottom right. */
function freeCorner(busy,w){
 const spots=[[51-w,1],[1,1],[51-w,10],[1,10]];
 for(const [x,y] of spots){let free=true;for(let j=y-1;j<=y+5&&free;j++)for(let i=x-1;i<=x+w;i++)if(busy(i,j)){free=false;break;}if(free)return [x,y];}
 return spots[0];
}
function counter(d,n,busy,c){const w=String(n).length*4-1,[x,y]=freeCorner(busy,w);d.rect(x-1,y-1,w+2,7,0);number(d,n,x,y,c.white);}

/* Liczydło: 24 rods, one per hour; beads stack in the order they were paid. */
function hours(d,v,c,t){
 const list=v.orders,ev=v.event,col=h=>2+2*h,nowX=1+2*v.now/3600,hourNow=Math.floor(v.now/3600);
 const stacks=Array.from({length:24},()=>[]);list.forEach((o,i)=>stacks[clamp(Math.floor(o.t/3600),0,23)].push(i));
 const tallest=Math.max(1,...stacks.map(s=>s.length)),pitch=Math.min(2,14/tallest);
 const place=i=>{const h=clamp(Math.floor(list[i].t/3600),0,23);return [col(h),14.5-(stacks[h].indexOf(i)+1)*pitch+pitch/2];};
 // LEDs make faint light look strong: hour marks stay very dim, only every sixth one is a real mark.
 for(let h=0;h<24;h++)if(h<=hourNow||h%6===0)d.dot(col(h),15,scale(c.white,h%6===0?(h<=hourNow?.3:.1):.06));
 // The live hour: a spark rises above its beads.
 const top=14.5-stacks[hourNow].length*pitch,rise=frac(t/4);
 if(top>2)d.dot(col(hourNow),Math.round(top-.5-(top-1.5)*rise),scale(c.white,.45*(1-rise)**2));
 d.rect(nowX,15,1,1,c.white);
 const last=list.length-1,s=ev?ev.s:99;
 list.forEach((o,i)=>{
  const colour=c.tiers[tier(o.v)];let [x,y]=place(i);
  if(i===last&&ev){
   if(s<.5){d.ellipse(x,0,.8,.8,scale(colour,s/.5),true);return;}
   if(s<1.3)y=mix(0,y,((s-.5)/.8)**2);
   else if(s<1.55)y-=.8*Math.sin(Math.PI*(s-1.3)/.25);
   if(s>=1.3&&s<2.2){const q=(s-1.3)/.9;d.ellipse(x,y,.8+2.6*q,.8+2.6*q,scale(colour,.7*(1-q)));}
  }
  if(pitch>=1.6)d.dot(x,ev&&i===last?y:Math.round(y),colour);else d.rect(x,y-pitch/2+.5,1,pitch,colour);
 });
 const shown=ev&&s<1.3?v.count-1:v.count;
 counter(d,shown,(x,y)=>{if(y>=15)return false;const h=(x-1)>>1;return h>=0&&h<24&&stacks[h].length>0&&y>=14.5-stacks[h].length*pitch-1;},c);
}

/* Gwiazdozbiór: across = hour of payment, up = value; lines join the stars in the order they appeared. */
function sky(d,v,c,t){
 const list=v.orders,ev=v.event,s=ev?ev.s:99,last=list.length-1,nowX=1.5+48*v.now/DAY;
 const star=o=>[Math.round(1.5+48*o.t/DAY),Math.round(13-11*weight(o.v))];
 for(let x=1;x<nowX;x+=2)d.dot(x,15,scale(c.white,x%12===1?.22:.06));
 d.rect(nowX,15,1,1,scale(c.white,.8));
 const lineColour=scale(c.tiers[0],.13);
 for(let i=1;i<list.length;i++){
  const [x0,y0]=star(list[i-1]),[x1,y1]=star(list[i]);
  if(i===last&&ev){if(s<2)continue;const q=part(s,2,2.9);d.line(x0,y0,x0+(x1-x0)*q,y0+(y1-y0)*q,lineColour);}
  else d.line(x0,y0,x1,y1,lineColour);
 }
 list.forEach((o,i)=>{
  const [x,y]=star(o),w=weight(o.v),colour=c.tiers[tier(o.v)],tw=.72+.28*Math.sin(t*(1.1+unit(i)*1.6)+unit(i+77)*TAU);
  let core=blend(colour,c.white,.25+.2*w),flash=0;
  if(i===last&&ev){
   if(s<1.6){
    // A comet brings the new star from the top right.
    const path=u=>{u=ease(u);const a=1-u;return [a*a*Math.min(56,nowX+14)+2*a*u*(x+8)+u*u*x,a*a*-3+2*a*u*-2+u*u*y];};
    const u=(s-.25)/1.35;if(u<0)return;
    for(let k=7;k>=0;k--){const [cx,cy]=path(Math.max(0,u-k*.035));d.rect(cx,cy,1,1,k?scale(c.white,.55*(1-k/8)):c.white);}
    return;
   }
   flash=1-part(s,1.6,2.8);
   if(s<2.6){const q=(s-1.6);d.ellipse(x,y,.6+3.2*q,.6+3.2*q,scale(colour,.6*(1-q)));}
  }
  core=blend(scale(core,tw),c.white,flash);
  if(w>=.35){const arm=scale(colour,(w>=.7?.55:.28)*tw+.4*flash);d.dot(x-1,y,arm);d.dot(x+1,y,arm);d.dot(x,y-1,arm);d.dot(x,y+1,arm);
   if(w>=.7){const tip=scale(colour,.18*tw+.3*flash);d.dot(x-2,y,tip);d.dot(x+2,y,tip);d.dot(x,y-2,tip);d.dot(x,y+2,tip);}}
  d.dot(x,y,core);
 });
 const shown=ev&&s<1.6?v.count-1:v.count;
 counter(d,shown,(x,y)=>{if(y>=15||x>=nowX+1)return false;return list.some((o,i)=>{const [sx,sy]=star(o);if(Math.abs(sx-x)<=2&&Math.abs(sy-y)<=2)return true;
  if(!i)return false;const [px,py]=star(list[i-1]);if(x<Math.min(px,sx)||x>Math.max(px,sx))return false;const yy=px===sx?py:py+(sy-py)*(x-px)/(sx-px);return Math.abs(yy-y)<1.2;});},c);
}

/* Słoik: marbles drop through the top and settle into a mound, simulated once per marble. */
const physics={key:'',balls:[],paths:[],columns:[]};
function marbles(list,total){
 // Marbles shrink in steps as the day fills up, so the whole pile is re-simulated only now and then.
 const k=Math.max(.4,Math.min(1,Math.floor(Math.sqrt(55/Math.max(1,total))*20)/20)),key=k+'|';
 if(physics.key!==key){physics.key=key;physics.balls=[];physics.paths=[];physics.columns=Array.from({length:W},()=>[]);}
 for(let i=physics.balls.length;i<list.length;i++){
  const r=Math.max(.5,k*(.85+1.25*weight(list[i].v))),dt=1/120,path=[];
  // Later marbles fall from a wider mouth, so the mound spreads instead of towering.
  let x=25.5+(unit(i*31+7)-.5)*Math.min(46,5+i*.8),y=-r-1,vx=(unit(i*17+3)-.5)*3,vy=4,calm=0,hit=-1;
  for(let step=0;step<720&&calm<24;step++){
   vy+=70*dt;if(step<40)vx+=(unit(i*977+step)-.5)*.4;x+=vx*dt;y+=vy*dt;let touching=false;
   if(y>15.5-r){y=15.5-r;if(vy>0)vy=-vy*.3;vx*=.92;touching=true;}
   if(x<r-.5){x=r-.5;vx=Math.abs(vx)*.4;}if(x>51.5-r){x=51.5-r;vx=-Math.abs(vx)*.4;}
   const cx=Math.round(x);for(let col=Math.max(0,cx-3);col<=Math.min(W-1,cx+3);col++)for(const b of physics.columns[col]){const dx=x-b.x,dy=y-b.y,m=r+b.r,d2=dx*dx+dy*dy;if(d2>=m*m)continue;
    const dd=Math.sqrt(d2)||1e-6,nx=dx/dd,ny=dy/dd;x=b.x+nx*m;y=b.y+ny*m;const vn=vx*nx+vy*ny;if(vn<0){vx-=1.3*vn*nx;vy-=1.3*vn*ny;}vx*=.993;vy*=.993;touching=true;}
   if(touching&&hit<0)hit=path.length;
   calm=touching&&vx*vx+vy*vy<.01?calm+1:0;
   if(step%2===0)path.push([x,y]);
  }
  const ball={x,y,r};physics.balls.push(ball);physics.columns[clamp(Math.round(x),0,W-1)].push(ball);physics.paths.push({path,hit:Math.max(0,hit)});
 }
 return physics.balls.slice(0,list.length);
}
function jar(d,v,c,t){
 const list=v.orders,ev=v.event,s=ev?ev.s:99,last=list.length-1,balls=marbles(list,v.count);
 // Marble colour follows the hour it was paid, light comes from where the sun is now.
 const stops=[c.hues[3]||c.tiers[0],c.tiers[1],c.tiers[2],c.tiers[0]];
 const hourColour=sec=>{const p=frac((sec/DAY*24-3)/24)*4,i=Math.floor(p);return blend(stops[i%4],stops[(i+1)%4],ease(p-i));};
 const sun=clamp((v.now/3600-6)/12,0,1)-.5,glint=Math.floor(t/1.7),glintBall=list.length?hash(glint)%list.length:-1,gq=Math.sin(Math.PI*frac(t/1.7));
 let first=null;
 balls.forEach((b,i)=>{
  let {x,y}=b;
  if(i===last&&ev){const {path,hit}=physics.paths[i],f=(s-.2)*60;if(f<0)return;const p=path[Math.min(path.length-1,Math.floor(f))];x=p[0];y=p[1];if(f>=hit)first=first??[x,y];}
  const colour=hourColour(list[i].t);
  d.ellipse(x,y,b.r,b.r,colour,true);
  if(b.r>=1.1)d.rect(x+sun*b.r*.9-.5,y-b.r*.45-.5,1,1,blend(colour,c.white,.55));
  if(i===glintBall&&!ev)d.rect(x+sun*b.r*.9-.5,y-b.r*.45-.5,1,1,blend(colour,c.white,.55+.45*gq));
 });
 const landed=!ev||(s-.2)*60>=physics.paths[last].hit;
 // The settled marble rings once, so even a short drop onto a full pile reads as an arrival.
 if(ev&&last>=0){const {path}=physics.paths[last],q=(s-.2-path.length/60)/.9;if(q>=0&&q<1){const b=balls[last],r=b.r+.6+2.4*q;d.ellipse(b.x,b.y,r,r,scale(hourColour(list[last].t),.75*(1-q)));}}
 counter(d,landed?v.count:v.count-1,(x,y)=>balls.some(b=>Math.abs(b.x-x)<=b.r+.5&&Math.abs(b.y-y)<=b.r+.5),c);
}

/* Tablica: split-flap count tiles and the two latest arrivals. */
function flapTile(d,x,from,to,q,ink,tile,shine){
 // The digit runs straight across the split; only the tile behind it has the gap, or a 5 reads as C over J.
 const hinge=7.5;
 const body=(rows,y0,y1,k,tint)=>{for(let y=y0;y<=y1;y++){
  const yy=hinge+(y-hinge)*k,r=y-3,bits=rows&&r>=0&&r<10?rows[r]:'.....';
  if(y!==8)d.rect(x,yy,7,k,scale(tile,tint));
  for(let i=0;i<7;i++){const on=i>0&&i<6&&bits[i-1]==='#',f=shine(x+i,y);
   if(on)d.rect(x+i,yy,1,k,scale(ink,tint));else if(f>0)d.rect(x+i,yy,1,k,scale(blend(tile,ink,.22*f),tint));}}};
 if(q>=1||from===to){body(to,0,7,1,1);body(to,8,15,1,1);return;}
 body(to,0,7,1,1);body(from,8,15,1,1);
 if(q<.5){const k=Math.cos(q*Math.PI);body(from,0,7,k,.55+.45*k);}else{const k=-Math.cos(q*Math.PI);body(to,8,15,k,.55+.45*k);}
}
function board(d,v,c,t){
 const list=v.orders,ev=v.event,s=ev?ev.s:99,n=v.count,tile=scale(c.white,.045),grey=scale(c.white,.2);
 const digits=k=>String(Math.min(999,Math.max(0,k))).padStart(3,'0'),zeros=k=>{const str=digits(k);let z=0;while(z<2&&str[z]==='0')z++;return z;};
 const now=digits(n),before=digits(Math.max(0,n-1));
 // A glint crosses the tiles every ten seconds; after an arrival it follows the flaps.
 const gs=ev?s-1.4:t%10,bx=-4+30*gs/1.4,shine=(x,y)=>gs<0||gs>1.4?0:Math.max(0,1-Math.abs(x-bx-(y-7)*.4)/1.5);
 for(let p=0;p<3;p++){
  const q=ev?part(s,.12*(2-p),.12*(2-p)+.6):1,z=zeros(q<.5?Math.max(0,n-1):n);
  flapTile(d,p*8,BIG[+before[p]],BIG[+now[p]],q,p<z?grey:c.white,tile,shine);
 }
 const rows=[list[list.length-1],list[list.length-2]],old=[list[list.length-2],list[list.length-3]];
 rows.forEach((o,r)=>{
  const y=r?9:2,ink=o?r?scale(c.white,.5):c.white:grey,from=old[r]?clock(old[r].t):'--:--',to=o?clock(o.t):'--:--',start=r?.35:.55;
  d.dot(39,y+1,ink);d.dot(39,y+3,ink);
  [0,1,3,4].forEach(i=>{const q=ev?part(s,start+.07*i,start+.07*i+.3):1,k=from[i]===to[i]?1:Math.abs(Math.cos(q*Math.PI));
   glyph(d,q<.5?from[i]:to[i],[31,35,0,41,45][i],y,ink,k);});
  const mq=ev?part(s,start,start+.3):1,was=old[r]?c.tiers[tier(old[r].v)]:0,mark=o?c.tiers[tier(o.v)]:0,m=mq<.5?scale(was,1-2*mq):scale(mark,2*mq-1);
  if(m)d.rect(27,y+1.5,2,2,m);
 });
}

/* Witraż: multiplicatively weighted Voronoi panes, curved leading between them, sunlight across the glass. */
const glassCache={key:'',colours:[],labels:null,edges:null};
const COLUMN_SIN=Array.from({length:W},(_,x)=>Math.sin(x*.23)),COLUMN_COS=Array.from({length:W},(_,x)=>Math.cos(x*.23));
function seeds(list,plaqueX){return list.map((o,i)=>{const x=1.5+49*frac(.5+i*.7548776662);let y=1.5+13*frac(.5+i*.569840291);if(x>plaqueX-2&&y>8)y-=8;return {x,y,s:.75+.6*weight(o.v)};});}
const diagram=()=>({labels:new Int16Array(W*H),edges:new Float32Array(W*H),qa:new Float32Array(W*H),qb:new Float32Array(W*H),sa:new Float32Array(W*H),sb:new Float32Array(W*H)});
function voronoi(points,out){
 for(let y=0;y<H;y++)for(let x=0;x<W;x++){let a=-1,qa=1e9,qb=1e9,sa=1,sb=1;
  for(let i=0;i<points.length;i++){const p=points[i],q=Math.sqrt((x-p.x)**2+(y-p.y)**2)/p.s;if(q<qa){qb=qa;sb=sa;qa=q;sa=p.s;a=i;}else if(q<qb){qb=q;sb=p.s;}}
  const k=y*W+x;out.labels[k]=a;out.qa[k]=qa;out.qb[k]=qb;out.sa[k]=sa;out.sb[k]=sb;out.edges[k]=qb<1e9?(qb-qa)*(sa+sb)/2:9;}
 return out;
}
// A growing seed on top of the finished diagram without it. Pixels are visited nearest first and the
// walk stops where the seed can no longer be first or second, so an arrival stays inside the frame budget.
function addSeed(base,p,index,out){
 if(base.seed!==p.x+','+p.y){base.seed=p.x+','+p.y;base.near=Array.from({length:W*H},(_,k)=>k).map(k=>[k,Math.hypot(k%W-p.x,Math.floor(k/W)-p.y)]).sort((a,b)=>a[1]-b[1]);
  base.reach=0;for(let k=0;k<W*H;k++)if(base.qb[k]<1e9)base.reach=Math.max(base.reach,base.qb[k]);else base.reach=1e9;}
 out.labels.set(base.labels);out.edges.set(base.edges);
 for(const [k,dist] of base.near){const q=dist/p.s;if(q>=base.reach)break;let a=base.labels[k],qa=base.qa[k],qb=base.qb[k],sa=base.sa[k],sb=base.sb[k];
  if(q<qa){qb=qa;sb=sa;qa=q;sa=p.s;a=index;}else if(q<qb){qb=q;sb=p.s;}else continue;
  out.labels[k]=a;out.edges[k]=qb<1e9?(qb-qa)*(sa+sb)/2:9;}
 return out;
}
function glass(d,v,c,t){
 const list=v.orders,ev=v.event,s=ev?ev.s:99,n=v.count,w=String(n).length*4-1,plaqueX=51-w-1;
 const pts=seeds(list,plaqueX),key=plaqueX+'|'+list.map(o=>o.t+':'+o.v).join(',');
 if(glassCache.key!==key){
  // Each pane takes a colour none of its neighbours had when it arrived, so old panes never change colour.
  const keep=glassCache.key.split('|')[0]===String(plaqueX)?glassCache.colours.slice(0,list.length):[];
  const tmp=diagram();
  for(let i=keep.length;i<list.length;i++){voronoi(pts.slice(0,i+1),tmp);const near=new Set();
   for(let y=0;y<H;y++)for(let x=0;x<W;x++){const a=tmp.labels[y*W+x];if(a!==i)continue;for(const [dx,dy] of [[1,0],[0,1],[-1,0],[0,-1]]){const xx=x+dx,yy=y+dy;if(xx>=0&&xx<W&&yy>=0&&yy<H&&tmp.labels[yy*W+xx]!==i)near.add(keep[tmp.labels[yy*W+xx]]);}}
   const start=hash(i+401)%5;let pickC=start;for(let k=0;k<5;k++){const cand=(start+k)%5;if(!near.has(cand)){pickC=cand;break;}}keep.push(pickC);}
  Object.assign(glassCache,voronoi(pts,diagram()),{key,colours:keep,before:list.length?voronoi(pts.slice(0,-1),diagram()):null,live:diagram(),frameAt:-1});
 }
 let field=glassCache,grow=1;const last=list.length-1,frameKey=key+'|'+n+(ev?'|arrival':''),at=ev?s:t;
 // The light drifts slowly and a pane grows over two seconds: relit 20 (30 while growing) times a second.
 if(glassCache.frame&&glassCache.frameKey===frameKey&&glassCache.framePalette===c&&at>=glassCache.frameAt&&at-glassCache.frameAt<(ev?1/30:.05))return glassCache.frame;
 if(ev&&last>=0&&s<2.4){grow=part(s,.2,2.2);field=addSeed(glassCache.before,{...pts[last],s:Math.max(.04,pts[last].s*grow)},last,glassCache.live);}
 // Whole pixels only, so the glass is written straight into the frame instead of through the rasterizer.
 const hour=v.now/3600,day=hour>6.5&&hour<19.5,sunX=-6+64*clamp((hour-6.5)/13),base=day?.34:.32,out=new Uint32Array(W*H);
 const column=Array.from({length:W},(_,x)=>base+(day?.5*Math.exp(-(((x-sunX)/15)**2)):.12)),phase=Array.from({length:H},(_,y)=>Math.sin(y*.5+t*.17)*1.4-t*.31);
 // Drifting cloud light sin(x·.23 + phase(y)), expanded so each pixel costs two multiplications.
 const rs=phase.map(Math.sin),rc=phase.map(Math.cos),fresh=ev&&last>=0?blend(c.white,c.hues[glassCache.colours[last]??0],part(s,.6,2.8)):0,empty=scale(c.white,.1);
 for(let y=0;y<H;y++)for(let x=0;x<W;x++){
  if(x>=plaqueX-1&&y>=9)continue;
  const k=y*W+x,e=field.edges[k];if(e<.4)continue;
  const i=field.labels[k],pane=i<0?empty:i===last&&ev?fresh:c.hues[glassCache.colours[i]??0];
  const q=Math.min(1,column[x]*(.86+.14*(COLUMN_SIN[x]*rc[y]+COLUMN_COS[x]*rs[y])))*(e<1.2?(e-.4)/.8:1);
  out[k]=((R(pane)*q)<<16)|((G(pane)*q)<<8)|(B(pane)*q);
 }
 Object.assign(glassCache,{frame:out,frameAt:at,frameKey,framePalette:c});
 const shown=String(ev&&s<1?Math.max(0,n-1):n);
 [...shown].forEach((ch,j)=>{const bits=SMALL[ch];for(let r=0;r<5;r++)for(let i=0;i<3;i++)if(bits[r*3+i]==='1')out[(10+r)*W+plaqueX+1+j*4+i]=c.white;});
 return out;
}

const renderers={hours,sky,jar,board,glass};
/* view: {orders:[{t,v}] paid so far (t = seconds since local midnight, v = value or null), count, now, event:{s}|null} */
function frame(id,view,theme={},t=0){
 if(!renderers[id])throw Error('Unknown scene '+id);
 const d=surface(),direct=renderers[id](d,view,palette(theme),t);return direct||d.finish();
}
function rgb(data){const out=new Uint8Array(W*H*3);for(let i=0;i<data.length;i++){out[i*3]=data[i]>>>16&255;out[i*3+1]=data[i]>>>8&255;out[i*3+2]=data[i]&255;}return out;}
g.UlanziOrders={W,H,DAY,scenes,frame,rgb,palette,weight};
// The compo loops (demos.js) draw with the same rasterizer and palette.
g.UlanziPixels={W,H,surface,palette,rgb,scale,blend,hsv,hueGap,clamp,ease,part,hash,unit,frac};
if(typeof module!=='undefined'&&module.exports)module.exports=g.UlanziOrders;
})(typeof globalThis!=='undefined'?globalThis:this);
