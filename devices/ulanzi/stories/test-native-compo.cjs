const fs=require('fs'),assert=require('assert/strict'),{spawn}=require('child_process');
const os=require('os'),path=require('path');
const base=__dirname+'/',demo=require('./demos.js');
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'ulanzi-aa-')),token='7'.repeat(64);
fs.writeFileSync(dir+'/tokens','control='+token+'\n',{mode:0o600});
const child=spawn(base+'storyd-host',[base+'renderer.js',base+'device.js',dir+'/tokens',dir+'/state.json','18878','--dry-run'],{stdio:['ignore','ignore','inherit']});
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function req(path,body){let r=await fetch('http://127.0.0.1:18878'+path,{method:body?'POST':'GET',headers:{Authorization:'Bearer '+token,'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});assert(r.ok);return path==='/frame'?Buffer.from(await r.arrayBuffer()):r.json();}
(async()=>{await wait(500);let status=await req('/status');let checked=0,maxError=0;for(const d of demo.catalogue)for(const t of [0,3.137,9.567]){await req('/demo',{id:d.id,time:t,paused:true,autoplay:false});await wait(150);let actual=await req('/frame'),expected=demo.rgb(demo.frame(d.id,t,status.theme));assert.equal(actual.length,expected.length);let error=Math.max(...actual.map((v,i)=>Math.abs(v-expected[i])));assert(error<=1,d.id+' @'+t+' error '+error);maxError=Math.max(maxError,error);checked++;}console.log({native_js_frames:checked,max_channel_error:maxError,status:await req('/status')});})().finally(()=>{child.kill();}).catch(e=>{console.error(e);process.exitCode=1;});
