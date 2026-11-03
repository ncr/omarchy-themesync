/* Native 52x16 story scenes. Every transported object has a carrier/support. */
(function(global){
  'use strict';
  const W=52,H=16;
  const C={black:0,amber:0xffa000,gold:0xffdc00,orange:0xff6000,red:0xff0000,
    cyan:0x00d8ff,teal:0x008080,blue:0x004080,violet:0x9000ff,pink:0xff0080,
    lime:0x60ff00,green:0x009000,brown:0x804000,cream:0xffe080};
  const font={0:['111','101','101','101','111'],1:['010','110','010','010','111'],
    2:['111','001','111','100','111'],3:['111','001','111','001','111'],4:['101','101','111','001','001'],
    5:['111','100','111','001','111'],6:['111','100','111','101','111'],7:['111','001','010','010','010'],
    8:['111','101','111','101','111'],9:['111','101','111','001','111']};
  const themes=[
    {id:'city',name:'Miasto po zmroku',tag:'Mieszkaniec wraca do domu',duration:18,
     calm:'Nieregularne, delikatne migotanie ciepłych okien. Nigdy nie gasną.',
     event:'Spacer do drzwi → wejście do domu → rozsuwa się zasłona.',
     growth:'1 wpłata = 1 okno · wysoki środkowy dom ma 3 piętra'},
    {id:'rail',name:'Mała kolej',tag:'Kolejarze dopinają wagony',duration:32,
     calm:'Tli się palenisko, od czasu do czasu unosi się para.',
     event:'Kolejarze wpychają wagon od prawej, spinają sprzęg i wnoszą paczkę.',
     growth:'1 wpłata = 1 paczka · wagony dochodzą przy 1., 5., 9., 13. i 17. wpłacie'},
    {id:'orbit',name:'Konstelacja dnia',tag:'Saturn i start małej rakiety',duration:26,
     calm:'Niewielki refleks na pierścieniu Saturna.',
     event:'Zapłon → lot do nowej gwiazdy → powrót do bazy.',
     growth:'1 wpłata = 1 gwiazda · 4 konstelacje po 5 gwiazd'},
    {id:'vinyl',name:'Klub winylowy',tag:'Gramofon i okładki z widocznymi płytami',duration:28,
     calm:'Płyta powoli się obraca, delikatnie świeci wskaźnik wzmacniacza.',
     event:'Kurier przynosi album i umieszcza go na ścianie ekspozycyjnej.',
     growth:'1 wpłata = 1 okładka z wysuniętym winylem · losowe miejsca i kolory'},
    {id:'maker',name:'Neonowy warsztat',tag:'Symetryczna suwnica i chwytak',duration:46,
     calm:'Chwytak czeka nad pustym stanowiskiem podawania.',
     event:'Podanie → chwyt → podniesienie → przejazd → odłożenie → powrót.',
     growth:'1 wpłata = 1 klocek · 5 stosów budowanych od dołu'},
    {id:'garden',name:'Ogród z lampionem',tag:'Wzrost roślin i zmienna pogoda',duration:60,
     calm:'Chmura płynie równym tempem przez cały ogród; krople i motyl.',
     event:'Chmura podlewa wybraną roślinę, a pod nią wyrasta nowa gałązka i kwiat.',
     growth:'1 wpłata = 1 kwiat · losowe rośliny, najpierw dół, potem góra'}
  ];
  const clamp=(v,a=0,b=1)=>Math.max(a,Math.min(b,v));
  const mix=(a,b,t)=>a+(b-a)*clamp(t);
  const smooth=t=>{t=clamp(t);return t*t*(3-2*t);};
  const between=(s,a,b)=>smooth((s-a)/(b-a));
  const hash=n=>{n=Math.imul(n^0x9e3779b9,0x85ebca6b);n=Math.imul(n^(n>>>13),0xc2b2ae35);return (n^(n>>>16))>>>0;};
  function shuffled(n,seed){const a=Array.from({length:n},(_,i)=>i);for(let i=n-1;i>0;i--){const j=hash(seed+i)%(i+1);[a[i],a[j]]=[a[j],a[i]];}return a;}
  const colours=[C.cyan,C.orange,C.violet,C.lime,C.red,C.gold,C.pink,C.teal];
  const colour=i=>colours[hash(i+1703)%colours.length];
  function make(){
    const data=new Uint32Array(W*H);
    const p=(x,y,c)=>{x=Math.round(x);y=Math.round(y);if(x>=0&&x<W&&y>=0&&y<H)data[y*W+x]=c;};
    const rect=(x,y,w,h,c)=>{for(let j=0;j<h;j++)for(let i=0;i<w;i++)p(x+i,y+j,c);};
    const points=(x0,y0,x1,y1)=>{
      x0=Math.round(x0);y0=Math.round(y0);x1=Math.round(x1);y1=Math.round(y1);
      const out=[],dx=Math.abs(x1-x0),sx=x0<x1?1:-1,dy=-Math.abs(y1-y0),sy=y0<y1?1:-1;let err=dx+dy;
      for(;;){out.push([x0,y0]);if(x0===x1&&y0===y1)break;const e=2*err;if(e>=dy){err+=dy;x0+=sx;}if(e<=dx){err+=dx;y0+=sy;}}
      return out;
    };
    const line=(x0,y0,x1,y1,c,q=1)=>{const a=points(x0,y0,x1,y1);a.slice(0,Math.max(1,Math.ceil(a.length*clamp(q)))).forEach(([x,y])=>p(x,y,c));};
    const outline=(x,y,w,h,c)=>{line(x,y,x+w-1,y,c);line(x,y+h-1,x+w-1,y+h-1,c);line(x,y,x,y+h-1,c);line(x+w-1,y,x+w-1,y+h-1,c);};
    const sprite=(x,y,rows,palette)=>rows.forEach((row,j)=>[...row].forEach((key,i)=>{if(palette[key]!==undefined)p(x+i,y+j,palette[key]);}));
    return {data,p,rect,line,outline,sprite,points};
  }
  function spark(d,x,y,t,palette=[C.gold,C.pink,C.cyan]){
    if(t<=0||t>=1)return;
    [[-2,-3],[2,-3],[-3,-1],[3,-1],[0,-4]].forEach(([vx,vy],i)=>{
      const u=(t-i*.035)/.82;if(u>0&&u<1)d.p(x+vx*u*2,y+vy*u*2+u*u*8,palette[i%palette.length]);
    });
  }
  const houseSpecs=[{x:0,top:7,rows:[12,9]},{x:11,top:6,rows:[11,8]},
    {x:22,top:3,rows:[11,8,5]},{x:33,top:6,rows:[11,8]},{x:44,top:9,rows:[11]}];
  const citySlots=houseSpecs.flatMap((h,g)=>h.rows.flatMap(y=>[1,5].map(dx=>({x:h.x+dx,y,g}))));
  const railSlots=Array.from({length:20},(_,i)=>({x:12+Math.floor(i/4)*8+i%2*4,y:9-Math.floor(i%4/2)*3,g:Math.floor(i/4)}));
  const orbitSlots=Array.from({length:20},(_,i)=>({x:14+Math.floor(i/5)%2*12+i%5*2,y:Math.floor(i/10)*8+(i%5%2?4:1)}));
  const albumPlaces=Array.from({length:20},(_,i)=>({x:22+i%5*6,y:Math.floor(i/5)*4}));
  const vinylSlots=shuffled(20,617).map(i=>albumPlaces[i]);
  // Twin uprights frame the work area. The even-width cargo hangs from a
  // half-pixel axis, with mirrored cable strands and mirrored gripper jaws.
  const makerSlots=Array.from({length:20},(_,i)=>({x:11+Math.floor(i/4)*6,y:14-i%4*2,g:Math.floor(i/4)}));
  const gardenSlots=[],plantLevels=[0,0,0,0,0];
  for(let i=0;i<20;i++){
    const available=[0,1,2,3,4].filter(g=>plantLevels[g]<4);
    const g=available[hash(i+907)%available.length],stage=plantLevels[g]++;
    const side=(stage%2)^(hash(g+23)%2);
    gardenSlots.push({x:g*10+side*5,y:stage<2?10:6,g,stage});
  }
  const slots={city:citySlots,rail:railSlots,orbit:orbitSlots,vinyl:vinylSlots,maker:makerSlots,garden:gardenSlots};
  function person(d,x,t,hand){
    d.p(x,11,C.cream);d.line(x,12,x,14,C.blue);
    const step=Math.floor(t*3)%2;d.p(x-1+step,15,C.blue);d.p(x+1-step,15,C.blue);
    if(hand){d.line(x,12,hand.x,hand.y,C.cream);d.p(hand.x,hand.y,C.cream);}
  }
  function windowColour(i,t){
    const phase=(t+hash(i+61)%41)%41;
    const flicker=phase<2.8?Math.pow(Math.sin(phase*5.2),2)*.13:0;
    const warmth=.02*Math.sin(t*.35+i*2.1);
    const r=Math.round(255*(1-flicker*.35));
    const g=Math.round(220*(1-flicker+warmth));
    return r<<16|Math.min(235,g)<<8;
  }
  function city(d,n,t,e){
    const s=(e??0)*18,idx=Math.max(0,n-1),target=citySlots[idx],placed=e===null||s>=11;
    for(const h of houseSpecs){
      d.line(h.x,15,h.x+7,15,C.brown);d.line(h.x,h.top,h.x,15,C.brown);d.line(h.x+7,h.top,h.x+7,15,C.brown);
      d.line(h.x+1,h.top-1,h.x+6,h.top-1,C.brown);d.line(h.x+2,h.top-2,h.x+5,h.top-2,C.brown);d.rect(h.x+3,14,2,2,0);
    }
    citySlots.forEach((a,i)=>{if(i<n&&(i!==idx||placed))d.rect(a.x,a.y,2,2,windowColour(i,t));});
    d.sprite(44,0,['.GGG.','GG...','GG...','.GGG.'],{G:C.gold});
    if(e!==null&&n>0){
      const door=houseSpecs[target.g].x+3;
      if(s<7){const x=mix(53,door,between(s,0,5));person(d,x,s);if(s>5)d.rect(door,14,2,2,C.orange);}
      if(s>=7&&s<11){d.rect(target.x,target.y,1+(s>9?1:0),2,C.gold);}
      if(s>=12)spark(d,target.x,target.y,(s-12)/6,[C.gold,C.orange,C.cream]);
    }
  }
  function railState(n,e){
    const s=(e??0)*32,before=Math.max(0,n-1),wagons=Math.ceil(n/4),oldWagons=Math.ceil(before/4);
    const needsWagon=wagons>oldWagons,target=railSlots[Math.max(0,n-1)];
    const carryStart=needsWagon?18:0,carryEnd=needsWagon?25:13,placeAt=needsWagon?28:22;
    const placed=e===null||s>=placeAt;
    const wagonX=needsWagon&&e!==null?mix(54,12+(wagons-1)*8,between(s,0,14)):12+(wagons-1)*8;
    const carrierStand=target.x+4;
    let carrierX=mix(55,carrierStand,between(s,carryStart,carryEnd));
    if(s>=placeAt)carrierX=mix(carrierStand,56,between(s,placeAt,32));
    return {s,offset:0,load:placed?n:before,wagons,oldWagons,needsWagon,wagonX,
      visibleWagons:e!==null&&needsWagon?oldWagons+(wagonX<52?1:0):wagons,
      coupled:e===null||!needsWagon||s>=18,placed,carryStart,carryEnd,placeAt,carrierX,
      cargoY:mix(target.y-2,target.y,between(s,carryEnd,placeAt))};
  }
  function railWorker(d,x,s,hand,pushing=false){
    const lean=pushing?-1:0;
    d.rect(x-1+lean,8,2,1,C.red);d.p(x+lean,9,C.cream);d.line(x,10,x,13,C.blue);
    const stride=Math.floor(s*3)%2;d.p(x-1+stride,14,C.blue);d.p(x+1-stride,15,C.blue);
    if(hand)d.line(x,11,hand.x,hand.y,C.cream);
  }
  function rail(d,n,t,e){
    const state=railState(n,e),s=state.s;
    d.line(0,15,51,15,C.brown);for(let x=1;x<52;x+=4)d.p(x,14,C.brown);
    // The locomotive and all coupled wagons remain parked throughout delivery.
    d.rect(1,10,10,3,C.cyan);d.rect(1,9,6,3,C.teal);d.outline(6,6,5,7,C.cyan);
    d.rect(7,7,2,2,C.gold);d.rect(2,7,2,2,C.teal);d.p(0,11,C.cream);
    d.p(8,11,0xff0000|Math.round(92+28*Math.sin(t*.7))<<8);d.p(9,11,C.red);
    for(const x of [2,6,10]){d.sprite(x-1,12,['.R.','RRR','.R.'],{R:C.red});d.p(x,13,C.gold);}
    function wagon(x,coupled,rolling=false){
      if(coupled)d.p(x-1,12,C.brown);d.line(x,12,x+6,12,C.cyan);
      for(const xx of [x+1,x+4]){d.rect(xx,13,2,2,C.blue);d.p(xx,rolling&&Math.floor(s*3)%2?13:14,C.cream);}
    }
    const fixed=e!==null&&state.needsWagon?state.oldWagons:state.wagons;
    for(let g=0;g<fixed;g++)wagon(12+g*8,true);
    if(e!==null&&state.needsWagon)wagon(state.wagonX,state.coupled,s<14);
    function parcel(x,y,i){const c=colours[(hash(Math.floor(i/4)+57)+i%4*3)%colours.length];d.sprite(x,y,['KKK','KkK','KKK'],{K:c,k:C.cream});}
    railSlots.forEach((a,i)=>{if(i<state.load)parcel(a.x,a.y,i);});
    const ph=t%19;
    if(ph<2.5){d.p(3+Math.floor(ph),6-Math.floor(ph),C.cream);if(ph>1){d.p(5,4,C.cream);d.p(6,3,C.blue);}}
    if(e!==null&&n>0){
      const a=railSlots[n-1],x=state.wagonX;
      if(state.needsWagon&&s<14){
        railWorker(d,x+2,s,{x:x+1,y:12},true);railWorker(d,x+5,s,{x:x+4,y:12},true);
      }else if(state.needsWagon&&s<18){
        railWorker(d,x-2,s,{x:x-1,y:12});railWorker(d,x+5,s,{x:x+6,y:12});d.p(x-1,12,C.gold);
      }else if(state.needsWagon&&s<25){
        for(const start of [x-2,x+5])railWorker(d,mix(start,56,between(s,18,25)),s);
      }
      if(s>=state.carryStart){
        if(!state.placed){parcel(state.carrierX-4,state.cargoY,n-1);railWorker(d,state.carrierX,s,{x:state.carrierX-1,y:state.cargoY+3});}
        else if(s<32)railWorker(d,state.carrierX,s);
      }
      if(s>state.placeAt)spark(d,a.x,a.y,(s-state.placeAt)/(32-state.placeAt));
    }
  }
  function rocket(d,x,y,flame,facing=1){
    const rows=['..R.....','.RRRR...','RRCRRRR.','.RRRR...','..R.....'];
    d.sprite(x,y,facing<0?rows.map(row=>[...row].reverse().join('')):rows,{R:C.red,C:C.cyan});
    if(flame){d.p(facing<0?x+8:x-1,y+2,C.gold);d.p(facing<0?x+9:x-2,y+2,C.orange);}
  }
  function orbit(d,n,t,e){
    const s=(e??0)*26,idx=Math.max(0,n-1),a=orbitSlots[idx];
    const shown=e!==null&&s<15?n-1:n;
    d.sprite(41,6,['..VVV..','.VVVVV.','VVvVVVV','VVvvVVV','VVVvVVV','.VVVVV.','..VVV..'],{V:C.violet,v:C.blue});
    d.line(38,11,51,7,C.orange);d.line(38,12,51,8,C.gold);
    orbitSlots.forEach((p,i)=>{if(i<shown&&i%5){const b=orbitSlots[i-1];d.line(b.x,b.y,p.x,p.y,C.blue);}});
    orbitSlots.forEach((p,i)=>{if(i<shown)d.sprite(p.x-1,p.y-1,['.S.','SSS','.S.'],{S:C.gold});});
    if(e===null){rocket(d,1,8,false);if(t%45<1.5)d.p(49,8,C.cream);}
    else{
      let x=1,y=8;
      if(s<5){rocket(d,x,y,s>1);return;}
      if(s<15){const q=between(s,5,15);x=mix(1,a.x-3,q);y=mix(8,Math.max(0,a.y-2),q);}
      else if(s<20){x=a.x-3;y=Math.max(0,a.y-2);spark(d,a.x,a.y,(s-15)/5);}
      else{x=mix(a.x-3,1,between(s,20,26));y=mix(Math.max(0,a.y-2),8,between(s,20,26));}
      rocket(d,x,y,s<15||s>20,s>20?-1:1);
    }
  }
  // A square sleeve, seen face-on, with a round LP partly pulled to the right.
  // No narrow vertical spines: the gold centre is the record's visible label.
  function album(d,x,y,i){d.sprite(x,y,['AAAA.','AAAK.','AAKOK','AAAK.'],{A:colour(i+80),K:C.blue,O:C.gold});}
  function vinyl(d,n,t,e){
    const s=(e??0)*28,idx=Math.max(0,n-1),a=vinylSlots[idx],placed=e===null||s>=18;
    // Featured "now playing" jacket: a larger round black LP makes the small
    // face-on jackets on the collection wall unambiguous at native resolution.
    d.rect(8,0,5,5,C.orange);
    d.sprite(10,0,['.RRR.','RKKKR','RKOKR','RKKKR','.RRR.'],{R:C.teal,K:0,O:C.gold});
    // Top deck is above the front fascia. The record is a flat 11x5 ellipse;
    // the tone-arm and stylus sit on that same top plane, never on the fascia.
    d.line(0,15,51,15,C.brown);d.rect(0,13,18,3,C.blue);
    d.line(0,13,17,13,C.violet);d.line(0,15,17,15,C.violet);
    d.line(0,10,3,7,C.violet);d.line(3,7,17,7,C.violet);d.line(17,7,17,13,C.violet);d.line(0,10,0,15,C.violet);
    d.sprite(2,8,['..BBBBBBB..','.BB.....BB.','BB...O...BB','.BB.....BB.','..BBBBBBB..'],{B:C.teal,O:C.gold});
    d.p(15,8,C.cream);d.line(15,8,14,10,C.cream);d.line(14,10,11,11,C.cream);d.p(11,12,C.gold);d.p(15,14,C.orange);
    const spin=e===null?t:s,positions=[[3,10],[4,9],[7,8],[10,9],[11,10],[10,11],[7,12],[4,11]];
    const [hx,hy]=positions[Math.floor(spin/1.7)%positions.length];d.p(hx,hy,C.cyan);
    d.p(15,14,0xff0000|Math.round(110+28*Math.sin(t*.65))<<8);
    d.outline(18,10,3,5,C.blue);d.p(19,12,C.teal);d.p(19,13,Math.floor(t/3)%2?C.teal:C.cyan);
    vinylSlots.forEach((p,i)=>{if(i<n&&(i!==idx||placed))album(d,p.x,p.y,i);});
    d.rect(16,3,5,1,C.orange);d.rect(17,1,3,2,C.orange);d.line(18,4,18,6,C.brown);
    if(e!==null&&n>0){
      const stand=a.x-2,feetTarget=Math.min(15,a.y+8);let px=stand,bx=a.x,by=a.y,feet=15;
      if(s<10){px=mix(53,stand,between(s,0,10));bx=px+2;by=8;}
      else if(s<18){feet=mix(15,feetTarget,between(s,10,15));bx=a.x;by=mix(8,a.y,between(s,10,18));}
      else if(s<21){feet=mix(feetTarget,15,between(s,18,21));}
      else if(s>21)px=mix(stand,55,between(s,21,28));
      if(s>=10&&s<21&&feetTarget<15){d.line(stand-1,feetTarget,stand-1,15,C.brown);d.line(stand+1,feetTarget,stand+1,15,C.brown);for(let y=feetTarget;y<15;y+=2)d.line(stand-1,y,stand+1,y,C.brown);}
      // A worker on a small stepladder can reach the upper display rows.
      const dy=feet-15;
      const actor=Object.assign({},d,{p:(x,y,c)=>d.p(x,y+dy,c),line:(x,y,x1,y1,c)=>d.line(x,y+dy,x1,y1+dy,c)});
      if(s<18){album(d,bx,by,idx);person(actor,px,s,{x:bx+2,y:by+4-dy});}
      else person(actor,px,s,s<21?{x:mix(a.x+2,stand+1,between(s,18,21)),y:mix(a.y+4,13+dy,between(s,18,21))-dy}:null);
    }
  }
  function craneState(n,e){
    const a=makerSlots[Math.max(0,n-1)],s=(e??1)*46;
    let cart=45.5,hook=2,cargoX=44,cargoY=14,attached=false,placed=e===null,grip=false,supply=false;
    if(e!==null){
      supply=s<13;
      if(s<5){cargoX=mix(54,44,between(s,0,4));}
      else if(s<11){hook=mix(2,12,between(s,5,11));}
      else if(s<13){hook=12;grip=true;}
      else if(s<21){hook=mix(12,2,between(s,13,21));attached=true;grip=true;}
      else if(s<29){cart=mix(45.5,a.x+1.5,between(s,21,29));attached=true;grip=true;}
      else if(s<35){cart=a.x+1.5;hook=mix(2,a.y-2,between(s,29,35));attached=true;grip=true;}
      else if(s<37){cart=a.x+1.5;hook=a.y-2;placed=true;grip=s<36;}
      else if(s<41){cart=a.x+1.5;hook=mix(a.y-2,2,between(s,37,41));placed=true;}
      else{cart=mix(a.x+1.5,45.5,between(s,41,46));placed=true;}
    }
    cart=Math.round(cart-.5)+.5;hook=Math.round(hook);
    if(attached){cargoX=cart-1.5;cargoY=hook+2;}
    const spread=grip?0:1;
    return {s,cart,hook,cargoX,cargoY,attached,placed,grip,supply,
      jaws:[cart-2.5-spread,cart+2.5+spread],cables:[cart-.5,cart+.5]};
  }
  function brick(d,x,y,i){const c=colour(i+25);d.rect(x,y,4,1,c);d.rect(x+1,y-1,2,1,c);}
  function maker(d,n,t,e){
    const st=craneState(n,e),idx=Math.max(0,n-1);
    d.line(0,15,51,15,C.brown);d.line(8,0,50,0,C.teal);
    for(const x of [8,50]){d.line(x,1,x,15,C.blue);d.rect(x-1,15,3,1,C.blue);}
    d.line(8,3,11,0,C.teal);d.line(50,3,47,0,C.teal);
    d.line(42,15,49,15,C.teal);
    makerSlots.forEach((p,i)=>{if(i<n&&(i!==idx||st.placed))brick(d,p.x,p.y,i);});
    if(e!==null&&n>0&&(st.supply||st.attached))brick(d,st.cargoX,st.cargoY,idx);
    d.rect(st.cart-1.5,1,4,1,C.pink);
    for(const x of st.cables)d.line(x,2,x,st.hook,C.gold);
    d.line(st.cart-2.5,st.hook,st.cart+2.5,st.hook,C.cream);
    for(const x of st.jaws)d.line(x,st.hook+1,x,st.hook+2,C.cream);
    if(e!==null&&st.s>35&&st.s<39){const a=makerSlots[idx];spark(d,a.x+1,a.y,(st.s-35)/4);}
  }
  function weatherState(time,focus){
    const p=((time%120)+120)%120;
    if(p<22||p>100)return {visible:false,p,rain:false};
    const destination=focus===undefined?14:clamp(focus-3,8,40);
    let x;
    if(focus===undefined)x=mix(54,-10,(p-22)/78);
    else if(p<36)x=mix(54,destination,between(p,22,36));
    else if(p<82)x=focus===undefined?mix(14,32,(p-36)/46):destination;
    else x=mix(focus===undefined?32:destination,56,between(p,82,100));
    const storm=p>=64&&p<82,rain=p>=36&&p<92;
    return {visible:true,p,x,storm,rain};
  }
  function weather(d,time,t,focus){
    const state=weatherState(time,focus),{p,x,storm,rain}=state;
    if(!state.visible)return state;
    d.sprite(x,0,['..CCC...','.CCCCC..','CCCCCCCC','.CCCCCC.'],{C:storm?C.blue:C.teal});
    if(rain){
      const drops=storm?8:5;
      for(let i=0;i<drops;i++){
        const age=(t*5+i*2.13)%12,dx=(hash(i+99)%8),dy=4+age;
        const rx=focus===undefined?x+dx:mix(x+dx,focus+(dx-4)*.4,(dy-4)/11);
        if(dy<15){d.p(rx,dy,C.cyan);if(storm&&i%2===0)d.p(rx,dy-1,C.blue);}
        else d.p(focus===undefined?x+dx:focus+(dx-4)*.4,14,C.cyan);
      }
    }
    const flash=storm&&((p>=65&&p<65.35)||(p>=74&&p<74.35));
    if(flash)d.sprite(x+3,3,['.Y.','YY.','.YY','Y..'],{Y:C.gold});
    return state;
  }
  function plantTop(list,g){return list.some(a=>a.g===g&&a.stage>=2)?8:list.some(a=>a.g===g)?12:14;}
  function gardenState(n,e){
    const s=(e??1)*60,a=gardenSlots[Math.max(0,n-1)],old=gardenSlots.slice(0,Math.max(0,n-1)),all=gardenSlots.slice(0,n);
    const before=plantTop(old,a.g),after=plantTop(all,a.g),branchStart=a.stage===2?16:12;
    let weatherTime=8;
    if(e!==null){
      if(s<6)weatherTime=10;
      else if(s<12)weatherTime=mix(22,36,(s-6)/6);
      else if(s<38)weatherTime=mix(36,82,(s-12)/26);
      else if(s<46)weatherTime=mix(82,101,(s-38)/8);
      else weatherTime=105;
    }
    return {s,target:a,old,all,before,after,currentTop:e===null?after:mix(before,after,between(s,10,20)),branchStart,
      branchProgress:e===null?1:between(s,branchStart,26),budProgress:e===null?1:clamp((s-28)/12),
      flowerDone:e===null||s>=40,weatherTime};
  }
  function garden(d,n,t,e,opt){
    const st=gardenState(n,e),{s,target:a,old,all}=st,idx=Math.max(0,n-1);
    const wt=opt.weatherTime??(e===null?t:st.weatherTime);
    const rainClock=opt.weatherTime===undefined?t+(e===null?0:s):opt.weatherTime/3;
    const atmosphere=weather(d,wt,rainClock,e===null?undefined:a.g*10+3);
    d.line(0,15,51,15,C.green);d.line(50,4,50,14,C.brown);d.outline(49,1,3,3,C.brown);d.p(50,2,C.gold);d.p(50,0,C.brown);
    for(let g=0;g<5;g++){
      const before=plantTop(old,g),after=plantTop(all,g);
      const top=e!==null&&g===a.g?st.currentTop:after;
      if(top<14){const x=g*10+3;d.line(x,14,x,top,C.green);if(top<13){d.p(x-1,13,C.lime);d.p(x+1,13,C.lime);}}
    }
    all.forEach((p,i)=>{
      const newFlower=i===idx&&e!==null,root=p.g*10+3;
      if(!newFlower||s>=st.branchStart){
        const q=newFlower?st.branchProgress:1;
        d.line(root,p.y+4,p.x+1,p.y+2,C.green,q);
        if(newFlower&&q<1){
          const path=d.points(root,p.y+4,p.x+1,p.y+2),tip=path[Math.max(0,Math.ceil(path.length*q)-1)];d.p(tip[0],tip[1],C.lime);
        }
      }
      const c=[C.gold,C.red,C.violet,C.pink,C.cyan][p.g];
      if(!newFlower||st.flowerDone)d.sprite(p.x,p.y,['.F.','FOF','.F.'],{F:c,O:C.orange});
      else if(s>=26){
        d.p(p.x+1,p.y+1,C.orange);
        const petals=[[1,0],[0,1],[2,1],[1,2]],k=Math.floor(st.budProgress*4);
        petals.slice(0,k).forEach(([x,y])=>d.p(p.x+x,p.y+y,c));
      }
    });
    if(e!==null&&s<6&&plantTop(old,a.g)===14)d.p(mix(a.x-6,a.g*10+3,between(s,0,6)),mix(4,14,between(s,0,6)),C.gold);
    if(e!==null&&s>=40&&s<47)spark(d,a.x+1,a.y,(s-40)/7);
    if(!atmosphere.rain&&(e===null||s>46)&&t%45<6){const q=t%45,x=12+q*5,y=1+Math.floor(q)%2;
      d.sprite(x,y,Math.floor(q*2)%2?['C.C','.V.','V.V']:['CC.CC','.CVC.','..V..'],{C:C.cyan,V:C.violet});}
  }
  const renderers={city,rail,orbit,vinyl,maker,garden};
  function frame(id,n=12,t=8,event=null,options={}){
    if(!renderers[id])throw new Error('Unknown scene '+id);
    n=clamp(Math.floor(n),0,20);if(event!==null&&event>=1)event=null;
    const d=make();renderers[id](d,n,t,event,options);
    if(options.counter!==false){const text=String(n);d.rect(0,0,text.length*4-1,5,0);
      [...text].forEach((c,j)=>font[c].forEach((row,y)=>[...row].forEach((bit,x)=>{if(bit==='1')d.p(j*4+x,y,C.amber);})));}
    return d.data;
  }
  const phases={
    city:[[5,'Powrót do domu'],[7,'Otwieranie drzwi'],[11,'Rozsuwanie zasłony'],[18,'Nowe światło w oknie']],
    rail:[[14,'Kolejarze wpychają wagon'],[18,'Spinanie sprzęgu'],[25,'Wnoszenie paczki'],[28,'Odkładanie na podparcie'],[32,'Kolejarze odchodzą']],
    orbit:[[5,'Zapłon silnika'],[15,'Lot do nowej gwiazdy'],[20,'Zapalenie gwiazdy'],[26,'Powrót rakiety']],
    vinyl:[[10,'Kurier niesie nowy album'],[18,'Wejście na drabinkę i zawieszenie okładki'],[21,'Zejście z drabinki'],[28,'Kurier wraca']],
    maker:[[5,'Podanie klocka na stanowisko'],[11,'Opuszczanie chwytaka'],[13,'Zamknięcie chwytu'],[21,'Podnoszenie klocka'],[29,'Przejazd nad stos'],[35,'Opuszczanie na podparcie'],[37,'Otwarcie chwytu'],[41,'Podnoszenie pustego chwytaka'],[46,'Powrót suwnicy']],
    garden:[[6,'Ziarenko / przygotowanie rośliny'],[12,'Chmura nad wybraną rośliną'],[20,'Podlewanie — rośnie łodyga lub gałązka'],[26,'Nowa gałązka i zawiązanie pąka'],[40,'Pąk rozwija płatki'],[47,'Kwitnienie — chmura odpływa'],[60,'Nowy kwiat i przejaśnienie']]
  };
  function describe(id,seconds,n=1){
    const seq=id==='rail'&&!railState(n,0).needsWagon?[[13,'Kolejarz wnosi paczkę'],[22,'Umieszczanie paczki na wagonie'],[32,'Kolejarz odchodzi']]:phases[id];
    return (seq.find(([end])=>seconds<end)||[0,'Spokojny widok'])[1];
  }
  function rgb(data){const out=new Uint8Array(W*H*3);data.forEach((c,i)=>{out[i*3]=c>>16&255;out[i*3+1]=c>>8&255;out[i*3+2]=c&255;});return out;}
  function anchors(id){return slots[id].map(a=>id==='garden'?[a.x+1,a.y+1]:id==='vinyl'?[a.x+3,a.y+2]:[a.x,a.y]);}
  global.UlanziConcepts={W,H,C,themes,frame,rgb,anchors,slots,describe,craneState,railState,gardenState,weatherState};
  if(typeof module!=='undefined'&&module.exports)module.exports=global.UlanziConcepts;
})(typeof globalThis!=='undefined'?globalThis:this);
