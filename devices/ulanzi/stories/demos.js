/* Omarchy / tiny compo. Pure 52×16 pixel scenes shared by QuickJS and the gallery. */
(function(g){
'use strict';
const W=52,H=16,TAU=Math.PI*2;
const catalogue=[
 ['boot','Boot Ritual','Terminalowy rytuał startu','terminal',16],
 ['bonsai','Jade Bonsai','Mały ogród zen w cyfrowym wietrze','pixel art',20],
 ['voxel','Voxel Quarter','Izometryczne miasto składane z klocków','architektura',18],
 ['warp','Hyperspace','Lot przez gwiazdy i neonowe bramy','demoscene',16],
 ['floor','Copper Floor','Miedziana szachownica i lewitujący kryształ','oldschool',16],
 ['liquid','Liquid Kernel','Płynne jądro zamknięte w konturach','matematyka',18],
 ['cafe','Rainy Café','Deszcz za oknem, para nad filiżanką','pixel art',20],
 ['fish','sudo aquarium','Ryby, bąbelki i terminalowy absurd','mały świat',18],
 ['stack','Stack Overflow','Autonomiczne klocki, które naprawdę lądują','automat',20],
 ['cube','Hypercube Club','Obracające się sześciany z czystych linii','wektory',16],
 ['orbit','Orbital Mechanics','Planetarium z mechaniczną precyzją','kosmos',20],
 ['silk','Silk Signal','Trzy wstęgi tkane z przesuniętych sinusów','matematyka',16],
 ['metro','Night Line','Metro, które zatrzymuje się po pasażera','mały świat',24],
 ['jazz','Midnight Quartet','Kontrabas, saksofon, perkusja i noc','muzyka bez dźwięku',20],
 ['drive','Pacific Drive','Nocna jazda pod niskim neonowym słońcem','synthwave',18],
 ['circuit','Circuit Bloom','Ścieżki drukowane budzą się jedna po drugiej','elektronika',20],
 ['windows','Window Ballet','Mała choreografia kafelkowego pulpitu','omarchy',20],
 ['moon','Moon Harbour','Podejście, lądowanie i rozładunek na Księżycu','kosmos',24],
 ['gears','Clockwork','Koła zębate i kulka w wiecznym obiegu','mechanika',18],
 ['forever','Omarchy Forever','Typografia, refleksy i ukłon na finał','finał',16]
].map((a,i)=>({id:a[0],name:a[1],description:a[2],genre:a[3],duration:a[4],number:i+1}));
const FONT={A:'010101111101101',B:'110101110101110',C:'011100100100011',D:'110101101101110',E:'111100110100111',F:'111100110100100',G:'011100101101011',H:'101101111101101',I:'111010010010111',J:'001001001101010',K:'101101110101101',L:'100100100100111',M:'101111111101101',N:'101111111111101',O:'010101101101010',P:'110101110100100',Q:'010101101111011',R:'110101110101101',S:'011100010001110',T:'111010010010010',U:'101101101101111',V:'101101101101010',W:'101101111111101',X:'101101010101101',Y:'101101010010010',Z:'111001010100111',0:'111101101101111',1:'010110010010111',2:'111001111100111',3:'111001111001111',4:'101101111001001',5:'111100111001111',6:'111100111101111',7:'111001010010010',8:'111101111101111',9:'111101111001111','>':'100010001010100','/':'001001010100100','-':'000000111000000','.':'000000000000010',':':'000010000010000','_':'000000000000111','+':'000010111010000','!':'010010010000010',' ':'000000000000000'};
const clamp=(v,a=0,b=1)=>Math.max(a,Math.min(b,v)),mix=(a,b,t)=>a+(b-a)*clamp(t),ease=t=>{t=clamp(t);return t*t*(3-2*t);},part=(t,a,b)=>ease((t-a)/(b-a));
function hash(n){n=Math.imul(n^0x9e3779b9,0x85ebca6b);return (n^(n>>>15))>>>0;}
const parseColour=(hex,fallback)=>typeof hex==='string'&&/^#?[0-9a-fA-F]{6}$/.test(hex)?parseInt(hex.replace('#',''),16):fallback;
function shade(c,q){return (Math.round((c>>>16&255)*q)<<16)|(Math.round((c>>>8&255)*q)<<8)|Math.round((c&255)*q);}
let cachedTheme=null,cachedPalette=null;
function palette(theme={}){
 if(theme===cachedTheme)return cachedPalette;
 const roles=theme.colors||{},a=parseColour(theme.accent,0x4cc9ff),b=parseColour(roles.magenta||theme.secondary,0xb97aff);
 cachedTheme=theme;cachedPalette={a,b,d:parseColour(roles.selection,shade(a,.3)),m:shade(a,.65),bd:shade(b,.35),white:parseColour(roles.foreground,0xd9e8ff),gold:parseColour(roles.yellow,0xffd000),orange:parseColour(roles.orange,0xff7000),red:parseColour(roles.red,0xff3040),green:parseColour(roles.green,0x40d060),blue:parseColour(roles.blue,0x0080ff),pink:parseColour(roles.magenta,b),black:0};
 return cachedPalette;
}
/* A 4×4 spatial sample grid per physical pixel. No image-space blur. */
function surface(aa=true){
 const S=aa?4:1,SW=W*S,SH=H*S,native=typeof nativeRasterBegin==='function';
 if(native)nativeRasterBegin(S);
 const high=native?null:new Uint32Array(SW*SH),low=native?null:new Uint32Array(W*H);
 function bounds(a,b,limit){return [Math.max(0,Math.ceil((a+.5)*S-.5)),Math.min(limit-1,Math.floor((b+.5)*S-.5))];}
 function mark(x,y,c){x=Math.round(x);y=Math.round(y);if(x>=0&&x<W&&y>=0&&y<H)low[y*W+x]=c;}
 function rect(x,y,w,h,c){
  if(native){nativeRasterRect(x,y,w,h,c);return;}
  if(w<=0||h<=0)return;const [x0,x1]=bounds(x-.5,x+w-.5,SW),[y0,y1]=bounds(y-.5,y+h-.5,SH);
  for(let yy=y0;yy<=y1;yy++)for(let xx=x0;xx<=x1;xx++){let px=(xx+.5)/S-.5,py=(yy+.5)/S-.5;if(px>=x-.5&&px<x+w-.5&&py>=y-.5&&py<y+h-.5)high[yy*SW+xx]=c;}
  for(let yy=Math.round(y);yy<y+h;yy++)for(let xx=Math.round(x);xx<x+w;xx++)mark(xx,yy,c);
 }
 function p(x,y,c){rect(x,y,1,1,c);}
 function line(x,y,xx,yy,c){
  if(native){nativeRasterLine(x,y,xx,yy,c);return;}
  const [x0,x1]=bounds(Math.min(x,xx)-.5,Math.max(x,xx)+.5,SW),[y0,y1]=bounds(Math.min(y,yy)-.5,Math.max(y,yy)+.5,SH),dx=xx-x,dy=yy-y,len=dx*dx+dy*dy;
  for(let j=y0;j<=y1;j++)for(let i=x0;i<=x1;i++){let px=(i+.5)/S-.5,py=(j+.5)/S-.5,q=len?clamp(((px-x)*dx+(py-y)*dy)/len):0,ex=px-x-q*dx,ey=py-y-q*dy;if(ex*ex+ey*ey<=.25)high[j*SW+i]=c;}
 }
 function box(x,y,w,h,c){line(x,y,x+w-1,y,c);line(x,y+h-1,x+w-1,y+h-1,c);line(x,y,x,y+h-1,c);line(x+w-1,y,x+w-1,y+h-1,c);}
 function ellipse(x,y,rx,ry,c,filled=false){
  if(native){nativeRasterEllipse(x,y,rx,ry,c,filled);return;}
  const [x0,x1]=bounds(x-rx-.5,x+rx+.5,SW),[y0,y1]=bounds(y-ry-.5,y+ry+.5,SH),r=Math.min(rx,ry);
  for(let j=y0;j<=y1;j++)for(let i=x0;i<=x1;i++){let dx=((i+.5)/S-.5-x)/rx,dy=((j+.5)/S-.5-y)/ry,v=Math.sqrt(dx*dx+dy*dy);if(filled?v<=1:Math.abs(v-1)*r<=.5)high[j*SW+i]=c;}
 }
 function text(str,x,y,c){[...str.toUpperCase()].forEach((k,i)=>[...(FONT[k]||FONT[' '])].forEach((v,j)=>{if(v==='1')p(x+i*4+j%3,y+Math.floor(j/3),c);}));}
 function sprite(x,y,rows,colors){rows.forEach((row,j)=>[...row].forEach((v,i)=>{if(colors[v]!==undefined)p(x+i,y+j,colors[v]);}));}
 function field(kind,t,a,b,white){
  if(native){nativeRasterField(kind,t,a,b,white);return;}
  const balls=Array.from({length:3},(_,i)=>[26+Math.sin(t*.7+i*2.1)*(12+i*2),7.5+Math.cos(t*.65+i*2.3)*4]),co=Math.cos(t*.45),si=Math.sin(t*.45);
  for(let j=0;j<SH;j++)for(let i=0;i<SW;i++){
   let x=(i+.5)/S-.5,y=(j+.5)/S-.5,c=0;
   if(kind===0){let v=0;for(let k=0;k<3;k++){let dx=(x-balls[k][0])*.75,dy=y-balls[k][1];v+=9/(dx*dx+dy*dy+1);}if(v>1.05&&v<1.8)c=a;else if(v>=1.8&&v<2.8)c=b;else if(v>4.8)c=white;}
   else if(y>=7.5){let z=10/(y-6),u=(x-26)*z*.12,v=z+t*.8;if((Math.floor(u*co-v*si)+Math.floor(u*si+v*co))%2===0)c=y>=11.5?a:b;}
   high[j*SW+i]=c;
  }
 }
 function sample(x,y){return native?nativeRasterSample(x,y):low[Math.round(y)*W+Math.round(x)];}
 function finish(){if(native)return nativeRasterFinish();const out=new Uint32Array(W*H),area=S*S;for(let y=0;y<H;y++)for(let x=0;x<W;x++){let r=0,g=0,b=0;for(let j=0;j<S;j++)for(let i=0;i<S;i++){let c=high[(y*S+j)*SW+x*S+i];r+=c>>>16&255;g+=c>>>8&255;b+=c&255;}out[y*W+x]=(Math.round(r/area)<<16)|(Math.round(g/area)<<8)|Math.round(b/area);}return out;}
 return {p,rect,line,box,ellipse,text,sprite,field,sample,finish};
}
function stars(d,t,c,n=16){for(let i=0;i<n;i++){let x=hash(i+15)%52,y=hash(i+81)%12;d.p(x,y,(Math.floor(t/3)+i)%9===0?c.white:c.d);}}
function person(d,x,y,c,phase=0){d.p(x,y,c.white);d.line(x,y+1,x,y+3,c.a);let k=Math.floor(phase*3)%2;d.p(x-1+k,y+4,c.b);d.p(x+1-k,y+4,c.b);}
const scenes={};
scenes.boot=(d,t,c)=>{const s=t%16;d.line(0,0,51,0,c.d);d.p(1,0,c.red);d.p(3,0,c.gold);d.p(5,0,c.green);
 const lines=['> OMARCHY','LOAD PIXELS','READY.'];let line=s<5?0:s<10?1:2, str=lines[line],chars=Math.min(str.length,Math.floor((s%5)*5));d.text(str.slice(0,chars),2,3,line===2?c.green:c.a);d.text(line===0?'52X16':line===1?'60 FPS':'MAKE ART',2,10,c.b);if(Math.floor(t*2)%2===0)d.rect(2+Math.min(11,chars)*4,7,3,1,c.white);for(let i=0;i<8;i++)if(s>3+i*.3)d.p(49,3+i,c.a);};
scenes.bonsai=(d,t,c)=>{d.ellipse(26,14,8,1,c.d,true);d.sprite(20,11,['CCCCCCCCCC','.CCCCCCCC.','..CCCCCC..'],{C:c.b});d.line(26,10,25,7,c.white);d.line(25,7,27,4,c.white);d.line(25,7,20,5,c.white);d.line(26,6,32,5,c.white);d.line(27,4,30,2,c.white);
 const q=t%20;[[18,4,5],[29,4,6],[25,2,4]].forEach(([x,y,r],i)=>{d.ellipse(x+Math.sin(t*.55+i)*.45,y,r,2,c.d,true);d.line(x-r+2,y-1,x+r-2,y-1,c.a);d.p(x+r-1,y,c.m);});let f=q/20;d.p(35+Math.sin(f*8)*3,2+f*11,c.gold);d.ellipse(44,3,2,2,c.b);d.line(0,15,51,15,c.d);};
function iso(d,x,y,h,c){for(let i=0;i<h;i++){d.line(x-3,y-i,x,y+2-i,c.d);d.line(x,y+2-i,x+4,y-i,c.m);}d.line(x-3,y-h,x,y-h-2,c.a);d.line(x,y-h-2,x+4,y-h,c.a);d.line(x-3,y-h,x,y-h+2,c.b);d.line(x,y-h+2,x+4,y-h,c.b);for(let j=2;j<h;j+=3)d.p(x+2,y-j,c.gold);}
scenes.voxel=(d,t,c)=>{for(let i=-2;i<10;i++){let x=i*9-(t*2)%9,y=13-(i%2)*2;d.line(x-5,y,x+4,y+4,c.d);}for(let i=0;i<7;i++){let x=i*9-((t*2)%9),h=3+hash(i+Math.floor(t*2/9))%7;iso(d,x,12-(i%2)*2,h,c);}d.p(44,2,c.white);};
const warpStars=Array.from({length:32},(_,i)=>({angle:hash(i+91)/4294967296*TAU,offset:hash(i+172)%100/100}));
scenes.warp=(d,t,c)=>{warpStars.forEach((a,i)=>{let z=(a.offset+t*.17)%1,r=.2+z*z*31,x=26+Math.cos(a.angle)*r*1.6,y=7.5+Math.sin(a.angle)*r*.55;d.line(x,y,26+Math.cos(a.angle)*(r+z*2)*1.6,7.5+Math.sin(a.angle)*(r+z*2)*.55,z>.7?c.white:i%2?c.a:c.b);});for(let j=0;j<3;j++){let q=(t*.24+j/3)%1,rx=2+q*q*32,ry=1+q*q*11;d.box(26-rx,7.5-ry,rx*2,ry*2,q<.7?c.d:c.m);}d.p(26,8,c.gold);};
scenes.floor=(d,t,c)=>{d.field(1,t,c.d,c.m,0);let y=4+Math.sin(t)*1.1;d.line(21,y,26,y-3,c.a);d.line(26,y-3,31,y,c.b);d.line(21,y,26,y+4,c.b);d.line(31,y,26,y+4,c.a);d.line(21,y,31,y,c.white);d.line(26,y-3,26,y+4,c.m);};
scenes.liquid=(d,t,c)=>d.field(0,t,c.a,c.b,c.white);
scenes.cafe=(d,t,c)=>{d.box(1,0,29,12,c.d);d.line(15,0,15,10,c.m);d.line(2,5,28,5,c.d);for(let i=0;i<13;i++){let x=3+hash(i+8)%25,y=(t*(1.8+(i%3)*.2)+hash(i+43)%12)%11;d.line(x,y,x-1,y+2,c.a);}d.line(0,15,51,15,c.b);d.sprite(32,9,['CCCCCCCC.','CCCCCC.CC','.CCCCCC.C','..CCCCCC.'],{C:c.white});d.line(31,13,43,13,c.a);for(let i=0;i<3;i++){let q=(t*.4+i/3)%1;d.p(35+i*3+Math.sin(q*6)*1.4,8-q*6,q>.6?c.d:c.m);}d.ellipse(47,3,2,2,c.gold,true);};
scenes.fish=(d,t,c)=>{d.box(0,0,52,16,c.d);d.text('> SUDO FISH',2,1,c.a);d.line(1,7,50,7,c.m);for(let i=0;i<3;i++){let x=((t*(i%2?-2:2)+i*19+80)%62)-5,y=9+i%2*3;let right=i%2===0;d.sprite(x,y,right?['.AAA..','AAAA.A','.AAA..']:['..AAA.','A.AAAA','..AAA.'],{A:i===1?c.gold:c.b});d.p(x+(right?3:2),y+1,0);}for(let i=0;i<5;i++){let y=14-(t*1.2+i*3)%7;d.p(5+i*10+Math.sin(t+i),y,c.a);}d.line(2,15,49,15,c.m);};
const blocks=[[[0,0],[1,0],[2,0],[2,1]],[[0,0],[1,0],[1,1],[2,1]],[[0,0],[1,0],[0,1],[1,1]],[[0,0],[1,0],[2,0],[1,1]],[[0,0],[0,1],[0,2],[0,3]]];
scenes.stack=(d,t,c)=>{d.box(0,0,23,16,c.d);let s=t%20,colors=[c.a,c.b,c.gold,c.green,c.orange],placements=[{x:0,y:5,k:0},{x:3,y:5,k:2},{x:6,y:5,k:1},{x:1,y:3,k:3},{x:5,y:3,k:0},{x:9,y:3,k:4}];let idx=Math.min(5,Math.floor(s/3));placements.forEach((b,i)=>{if(i>idx)return;let progress=clamp((s-i*3)/2.1),y=i===idx?mix(-4,b.y,ease(progress)):b.y;blocks[b.k].forEach(([xx,yy])=>d.rect(2+(b.x+xx)*2,1+(y+yy)*2,2,2,colors[i%5]));});d.text('STACK',28,2,c.a);d.text(String(Math.floor(s/3)).padStart(2,'0'),32,9,c.b);if(s>18){d.rect(1,13,21,2,Math.floor(s*6)%2?c.white:c.a);} };
const cubePoints=[[-1,-1,-1],[1,-1,-1],[1,1,-1],[-1,1,-1],[-1,-1,1],[1,-1,1],[1,1,1],[-1,1,1]],edges=[[0,1],[1,2],[2,3],[3,0],[4,5],[5,6],[6,7],[7,4],[0,4],[1,5],[2,6],[3,7]];
scenes.cube=(d,t,c)=>{for(let k=0;k<3;k++){let angle=t*.55+k*.7,ca=Math.cos(angle),sa=Math.sin(angle),cb=Math.cos(t*.3),sb=Math.sin(t*.3),cx=9+k*17;let p=cubePoints.map(([x,y,z])=>{let xx=x*ca+z*sa,zz=-x*sa+z*ca,yy=y*cb-zz*sb;zz=y*sb+zz*cb;return [cx+xx*4.6/(1+zz*.15),7.5+yy*4.6/(1+zz*.15),zz];});edges.forEach(([a,b])=>d.line(p[a][0],p[a][1],p[b][0],p[b][1],(p[a][2]+p[b][2])<0?c.a:c.bd));d.p(cx,8,c.white);}};
scenes.orbit=(d,t,c)=>{d.ellipse(25,8,3,3,c.gold,true);for(let i=0;i<3;i++){let rx=8+i*7,ry=2+i*2;for(let j=0;j<30;j++){let a=j/30*TAU;d.p(25+Math.cos(a)*rx,8+Math.sin(a)*ry,c.d);}let a=t*(.7-i*.16)+i*2,x=25+Math.cos(a)*rx,y=8+Math.sin(a)*ry;d.ellipse(x,y,i===2?2:1,1,i===1?c.b:c.a,true);if(i===2)d.line(x-3,y+1,x+3,y-1,c.white);} };
scenes.silk=(d,t,c)=>{for(let k=2;k>=0;k--){let last=null;for(let x=0;x<52;x++){let y=7.5+Math.sin(x*.15+t*.65+k*2.1)*4+Math.sin(x*.31-t*.4+k)*1.5;d.line(x,y,x,y+2,k===0?c.a:k===1?c.b:c.d);if(last&&x%3===0)d.line(x,y,last[0],last[1],c.m);last=[x,y];}}};
scenes.metro=(d,t,c)=>{let s=t%24;d.line(0,13,51,13,c.m);d.line(0,15,51,15,c.d);d.p(47,2,c.gold);d.line(47,3,47,11,c.d);d.text('N',44,5,c.b);let x=s<5?mix(56,4,part(s,0,5)):s<16?4:mix(4,-60,part(s,16,23));d.rect(x,5,43,7,c.d);d.line(x,5,x+41,5,c.a);d.line(x,11,x+42,11,c.b);for(let i=0;i<5;i++)d.rect(x+2+i*8,6,5,3,c.m);d.rect(x+18,6,6,5,0);let open=s>=7&&s<14;d.rect(x+18,6,open?1:3,5,c.white);d.rect(x+(open?23:21),6,open?1:3,5,c.white);for(let p of [7,13,31,37])d.rect(x+p,12,2,1,c.white);let px=s<9?27:s<12?mix(27,x+21,part(s,9,12)):null;if(px!==null)person(d,px,8,c,s);};
scenes.jazz=(d,t,c)=>{d.line(0,15,51,15,c.d);person(d,3,6,c,t);d.sprite(0,11,['WWWWWW','B.B.B.','BBBBBB','B....B'],{W:c.white,B:c.d});d.line(3,8,5,10,c.white);for(let x of [8,25,43]){d.line(x-4,0,x+4,0,c.d);d.p(x,1,c.m);}let beat=Math.sin(t*3.2);person(d,8,5,c,t);d.ellipse(13,11,3,4,c.b,true);d.line(13,3,13,14,c.white);d.line(8,8,13,7+beat,c.gold);person(d,25,5+Math.round(beat*.35),c,t);d.sprite(27,8,['GG..','.GG.','..GG','.GGG'],{G:c.gold});d.line(24,7,27,9,c.white);d.ellipse(42,12,4,3,c.b);d.line(38,7,46,7,c.gold);d.line(42,5,42,9,c.d);d.p(42,4,c.white);d.line(41,8,38,9+beat,c.white);d.line(43,8,46,9-beat,c.white);for(let i=0;i<4;i++){let q=(t*.28+i/4)%1,x=18+i*8+Math.sin(q*5)*2,y=8-q*8;d.p(x,y,c.a);d.line(x+1,y-2,x+1,y,c.a);} };
scenes.drive=(d,t,c)=>{d.ellipse(26,3,7,4,c.orange,true);for(let y=2;y<7;y+=2)d.line(19,y,33,y,0);let peaks=[[0,7],[7,3],[12,8],[20,6],[26,8],[35,5],[41,8],[47,4],[51,7]];for(let i=1;i<peaks.length;i++)d.line(...peaks[i-1],...peaks[i],c.b);for(let y=8;y<16;y++){let w=(y-6)*2.2,bend=Math.sin(t*.35)*((y-7)/8)*6;d.p(26-w+bend,y,c.a);d.p(26+w+bend,y,c.a);if((Math.floor(y+t*5)%5)<2)d.p(26+bend,y,c.gold);}d.sprite(19+Math.sin(t*.35)*2,12,['..CCCCCC..','.CCCCCCCC.','CCWCCCCWCC','RRCCCCCCRR'],{C:c.b,W:c.white,R:c.red});};
const routes=[[[1,2],[10,2],[14,6],[25,6]],[[3,13],[13,13],[18,8],[25,8]],[[51,2],[40,2],[35,6],[28,6]],[[50,14],[40,14],[34,8],[28,8]],[[20,0],[20,3],[25,3],[25,5]],[[32,15],[32,12],[27,12],[27,10]]];
scenes.circuit=(d,t,c)=>{d.box(23,5,7,6,c.d);d.sprite(25,7,['AA','AA'],{A:c.white});routes.forEach((r,i)=>{let q=(t*.12+i*.13)%1,segments=r.length-1;for(let j=0;j<segments;j++){d.line(...r[j],...r[j+1],c.d);let p=clamp(q*segments-j);if(p>0)d.line(...r[j],mix(r[j][0],r[j+1][0],p),mix(r[j][1],r[j+1][1],p),i%2?c.a:c.b);}d.ellipse(r[0][0],r[0][1],1,1,i%2?c.b:c.a);});for(let i=0;i<4;i++){d.p(22,6+i*1.5,c.gold);d.p(30,6+i*1.5,c.gold);} };
function window(d,x,y,w,h,c,kind=0){d.rect(x,y,w,h,0);d.box(x,y,w,h,c.d);d.line(x,y+1,x+w-1,y+1,c.a);d.p(x+1,y+1,c.red);if(kind===0){for(let i=0;i<3;i++)d.line(x+2,y+3+i*2,x+Math.min(w-2,3+(i*3)%Math.max(3,w-3)),y+3+i*2,i%2?c.b:c.m);}else if(kind===1){for(let i=0;i<5;i++)d.rect(x+2+i*2,y+h-2-(i%3+1),1,i%3+1,c.b);}else d.text('>_',x+2,y+3,c.a);}
scenes.windows=(d,t,c)=>{let s=t%20,q=part(s,3,7),r=part(s,12,16),left=mix(31,23,q),top=mix(9,6,r);window(d,0,0,left,16,c,0);window(d,left+1,0,51-left,top,c,1);window(d,left+1,top+1,51-left,15-top,c,2);let px=s<10?mix(44,left,part(s,1,7)):mix(left,43,part(s,11,17)),py=s<10?mix(4,9,part(s,1,7)):mix(9,top,part(s,11,17));d.sprite(px,py,['W..','WW.','WWW','.W.'],{W:c.white});};
scenes.moon=(d,t,c)=>{stars(d,t,c,12);d.line(0,15,51,15,c.d);d.ellipse(9,15,5,2,c.m);d.ellipse(44,15,6,2,c.d);d.sprite(1,8,['.AA...','AAA...','.A....','.A..A.','.AAAA.','....A.'],{A:c.b});let s=t%24,x=s<10?mix(52,29,part(s,0,10)):s<18?29:mix(29,52,part(s,18,24)),y=s<10?mix(1,9,part(s,0,10)):s<18?9:mix(9,-5,part(s,18,24));d.sprite(x,y,['..AAA..','.AAAAA.','AAWWAAA','.AAAAA.','A.....A'],{A:c.a,W:c.white});if(s<10||s>18){let flame=Math.floor(t*5)%2;d.line(x+2,y+4,x+2,y+5+flame,c.orange);d.line(x+4,y+4,x+4,y+5+flame,c.gold);}if(s>=12&&s<17){let px=mix(31,21,part(s,12,17));d.rect(px,12,3,3,c.gold);person(d,px-2,10,c,t);}d.line(17,6,17,14,c.white);d.sprite(18,6,['BBBB','BBB.'],{B:c.b});};
scenes.gears=(d,t,c)=>{const gear=(x,y,r,phase,col)=>{d.ellipse(x,y,r-1,r-1,col);for(let i=0;i<8;i++){let a=i/8*TAU+phase;d.p(x+Math.cos(a)*r,y+Math.sin(a)*r,col);}d.p(x,y,c.white);};gear(10,9,5,t*.8,c.a);gear(19,9,4,-t*.8,c.b);gear(26,10,3,t*.8,c.a);d.line(29,14,50,14,c.m);d.line(31,6,48,2,c.d);let s=t%18,x,y;if(s<7){x=mix(48,31,s/7);y=mix(1,5,s/7);}else if(s<10){x=31;y=mix(5,12,(s-7)/3);}else if(s<15){x=mix(31,49,(s-10)/5);y=12;}else{x=49;y=mix(12,1,(s-15)/3);}d.ellipse(x,y,1,1,c.gold,true);for(let i=0;i<7;i++)d.p(31+i*3+(t%1),15,c.d);d.line(50,2,50,14,c.b);};
scenes.forever=(d,t,c)=>{stars(d,t,c,13);d.text('OMARCHY',12,4,c.a);d.text('MAKE ART',10,11,c.b);let x=(t*7)%68-8;for(let y=4;y<9;y++)for(let xx=Math.max(0,Math.floor(x));xx<Math.min(52,x+2);xx++)if(d.sample(xx,y))d.p(xx,y,c.white);let q=(t%16)/16;d.line(3,9,3+q*45,9,c.d);d.p(2,5+Math.sin(t),c.gold);d.p(49,5-Math.sin(t),c.gold);};
function frame(id,t=0,theme={},options={}){const meta=catalogue.find(s=>s.id===id);if(!meta)throw Error('Unknown demo '+id);const d=surface(options.antialias!==false),c=palette(theme);t=Math.max(0,t);if(options.title&&t<1.15){d.text(String(meta.number).padStart(2,'0'),1,1,c.a);let words=meta.name.toUpperCase().split(' '),line=words[0].slice(0,11),second=words.slice(1).join(' ').slice(0,12);d.text(line,11,1,c.b);d.text(second||'TINY COMPO',2,9,c.a);}else scenes[id](d,options.title?t-1.15:t,c);return d.finish();}
function rgb(data){const a=new Uint8Array(W*H*3);for(let i=0;i<data.length;i++){a[i*3]=data[i]>>>16&255;a[i*3+1]=data[i]>>>8&255;a[i*3+2]=data[i]&255;}return a;}
g.UlanziDemos={W,H,catalogue,frame,rgb,palette};if(typeof module!=='undefined'&&module.exports)module.exports=g.UlanziDemos;
})(typeof globalThis!=='undefined'?globalThis:this);
