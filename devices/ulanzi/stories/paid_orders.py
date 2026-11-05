#!/usr/bin/env python3
"""Read-only BaseLinker daily fully-paid transitions -> local TC002 scenes.
No buyer fields are retained; tokens never enter logs or the device.
"""
import argparse, datetime as dt, decimal, fcntl, json, os, time, urllib.request, urllib.parse, subprocess
from pathlib import Path
from zoneinfo import ZoneInfo
TZ=ZoneInfo('Europe/Warsaw')
D=decimal.Decimal

def day_window(now=None):
    local=dt.datetime.fromtimestamp(time.time() if now is None else now,TZ)
    day=local.date();start=dt.datetime.combine(day,dt.time.min,TZ);end=dt.datetime.combine(day+dt.timedelta(days=1),dt.time.min,TZ)
    return day.isoformat(),int(start.timestamp()),int(end.timestamp())

def atomic(path,value):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True);tmp=path.with_suffix('.tmp')
    fd=os.open(tmp,os.O_CREAT|os.O_TRUNC|os.O_WRONLY,0o600)
    with os.fdopen(fd,'w') as f:json.dump(value,f,separators=(',',':'));f.flush();os.fsync(f.fileno())
    os.replace(tmp,path)

def summary(o):
    total=D(str(o.get('delivery_price',0)))+sum((D(str(p.get('price_brutto',0)))*D(str(p.get('quantity',0))) for p in o.get('products',[])),D(0))
    return {'id':int(o['order_id']),'created':int(o.get('date_add',0)),'confirmed':int(o.get('date_confirmed',0)),
            'changed':int(o.get('date_in_status',0)),'status':int(o.get('order_status_id',0)),
            'paid':str(o.get('payment_done',0)),'total':str(total.quantize(D('.01')))}

def fully_paid(o):return D(o['total'])>0 and D(o['paid'])>=D(o['total'])

def first_full_payment(payments):
    transitions=[]
    for p in payments:
        before,after,total=(D(str(p.get(k,0))) for k in ('paid_before','paid_after','total_price'))
        # An actual increase through the full-payment boundary; excludes partials and price reductions.
        if total>0 and before<total<=after and after>before:transitions.append(int(p['date']))
    return min(transitions) if transitions else None

class BaseLinker:
    def __init__(self,path,interval=2.5):self.token=Path(path).read_text().strip();self.interval=interval;self.last=0
    def call(self,method,params):
        # This integration has no API mutation methods, even when the source token permits more.
        if method not in ('getOrders','getJournalList','getOrderPaymentsHistory'):raise ValueError('read-only API')
        delay=self.last+self.interval-time.monotonic()
        if delay>0:time.sleep(delay)
        body=urllib.parse.urlencode({'method':method,'parameters':json.dumps(params)}).encode()
        req=urllib.request.Request('https://api.baselinker.com/connector.php',data=body,headers={'X-BLToken':self.token,'User-Agent':'ThemeSync-Ulanzi/1.0'})
        self.last=time.monotonic()
        with urllib.request.urlopen(req,timeout=20) as response:data=json.load(response)
        if data.get('status')!='SUCCESS':raise RuntimeError('BaseLinker '+str(data.get('error_code','unknown error')))
        return data
    def orders(self,params):
        # Inclusive ID cursor handles 100 orders sharing the same confirmation second.
        cursor=0
        while True:
            page=self.call('getOrders',dict(params,**({'id_from':cursor} if cursor else {}))).get('orders',[])
            for order in page:yield summary(order)
            if len(page)<100:return
            next_id=max(int(o['order_id']) for o in page)+1
            if next_id<=cursor:raise RuntimeError('BaseLinker non-advancing order page')
            cursor=next_id

class Bridge:
    def __init__(self,config,api=None):
        self.config=config;self.api=api or BaseLinker(config['baselinker_token_file']);self.path=Path(config['state_file'])
        try:self.state=json.loads(self.path.read_text())
        except FileNotFoundError:self.state={}
        self.state.setdefault('observed',{});self.state.setdefault('paid',{});self.state.setdefault('journal_cursor',1);self.state.setdefault('revision',0)
        self.last_full_scan=time.monotonic() if self.state.get('bootstrap_complete') and time.time()-self.state.get('last_success',0)<300 else 0;self.first=True;self.checked_histories=0
    def rollover(self):
        day,start,end=day_window()
        if self.state.get('day')!=day:
            self.state.update(day=day,paid={},revision=max(self.state['revision']+1,int(time.time()*1000)))
        return start,end
    def consider(self,o,start,end,force=False):
        key=str(o['id']);old=self.state['observed'].get(key,{})
        record={'paid':o['paid'],'total':o['total'],'created':o['created'],'first_full':old.get('first_full')}
        fingerprint=(o['paid'],o['total']);changed=fingerprint!=(old.get('paid'),old.get('total'))
        if fully_paid(o) and (force or changed or old.get('first_full') is None):
            payments=self.api.call('getOrderPaymentsHistory',{'order_id':o['id'],'show_full_history':True}).get('payments',[])
            record['first_full']=first_full_payment(payments);self.checked_histories+=1
        self.state['observed'][key]=record
        paid_at=record['first_full']
        if fully_paid(o) and paid_at is not None and start<=paid_at<end:self.state['paid'].setdefault(key,paid_at)
    def journal(self,start,end):
        while True:
            cursor=self.state['journal_cursor']
            logs=self.api.call('getJournalList',{'last_log_id':max(1,cursor),'logs_types':[3]}).get('logs',[])
            if not logs:return
            for log in sorted(logs,key=lambda x:int(x.get('id',x.get('log_id',0)))):
                event_id=int(log.get('id',log.get('log_id',0)))
                if start<=int(log['date'])<end:
                    for order in self.api.orders({'order_id':int(log['order_id'])}):self.consider(order,start,end,force=True)
                self.state['journal_cursor']=max(self.state['journal_cursor'],event_id+1)
            if self.state['journal_cursor']<=cursor:raise RuntimeError('BaseLinker non-advancing journal page')
    def scan(self,bootstrap=False):
        before=(self.state.get('day'),len(self.state['paid']))
        start,end=self.rollover();candidates={}
        full=(bootstrap and not self.state.get('bootstrap_complete')) or time.monotonic()-self.last_full_scan>300
        since=start-self.config.get('backfill_days',90)*86400 if full else start
        for o in self.api.orders({'date_from':since,'get_unconfirmed_orders':False}):
            # Recent changed terminal orders + all today's orders; old active orders below.
            if o['created']>=start or o['changed']>=start:candidates[o['id']]=o
        for status in self.config['active_status_ids'] if full else self.config.get('fast_status_ids',self.config['active_status_ids']):
            for o in self.api.orders({'status_id':status,'get_unconfirmed_orders':False}):candidates[o['id']]=o
        for o in candidates.values():self.consider(o,start,end)
        self.journal(start,end)
        if full:self.last_full_scan=time.monotonic()
        after=(self.state['day'],len(self.state['paid']))
        if before!=after or not self.state['revision']:self.state['revision']=max(self.state['revision']+1,int(time.time()*1000))
        self.state['last_success']=int(time.time());self.state['source']='payment_history+event_journal';self.state['bootstrap_complete']=True
        # Keep IDs/payment summaries for 100 days, never customer records.
        cutoff=start-100*86400
        self.state['observed']={k:v for k,v in self.state['observed'].items() if v.get('created',start)>=cutoff or k in self.state['paid']}
        atomic(self.path,self.state)
        return {'day':self.state['day'],'count':len(self.state['paid']),'orders':self.orders(),'revision':self.state['revision'],'asof':self.state['last_success'],'animate':not bootstrap}
    def orders(self):
        # Only the local time of day it was paid and its value in whole złoty; the clock keeps the latest 100.
        out=[]
        for key,paid_at in sorted(self.state['paid'].items(),key=lambda kv:(kv[1],kv[0])):
            local=dt.datetime.fromtimestamp(paid_at,TZ);total=self.state['observed'].get(key,{}).get('total')
            out.append([local.hour*3600+local.minute*60+local.second,int(D(total).quantize(D(1))) if total is not None else None])
        return out[-100:]
    def device(self,path,data=None,method=None):
        token_path=Path(self.config['ulanzi_token_file']);token=next(l[8:] for l in token_path.read_text().splitlines() if l.startswith('control='))
        headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'}
        req=urllib.request.Request('http://'+self.config['device_host']+':8878'+path,data=json.dumps(data).encode() if data is not None else None,method=method,headers=headers)
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=5) as response:return json.load(response)
    def deliver(self,payload):
        try:status=self.device('/status')
        except (urllib.error.URLError,TimeoutError,OSError):
            recovery=self.config.get('runtime_recovery')
            if not recovery:raise
            subprocess.run(recovery,check=True,timeout=240,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            status=self.device('/status')
        payload=dict(payload,animate=payload['animate'] and status['day']==payload['day'] and status['revision']>0)
        result=self.device('/state',payload,'PUT')
        if result['day']!=payload['day'] or result['count']!=payload['count']:raise RuntimeError('device count verification failed')
        atomic(self.config['status_file'],{'connected':True,'day':result['day'],'paid_today':result['count'],'scene':result['scene'],'synced_at':int(time.time()),'revision':result['revision']})
        return result

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--config',type=Path,required=True);parser.add_argument('--once',action='store_true');parser.add_argument('--scan-only',action='store_true');args=parser.parse_args()
    os.umask(0o077);config=json.loads(args.config.read_text());Path(config['state_file']).parent.mkdir(parents=True,exist_ok=True)
    lock=open(config['state_file']+'.lock','w');fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    b=Bridge(config);retry=5
    while True:
        start=time.monotonic()
        try:
            payload=b.scan(bootstrap=b.first)
            print(json.dumps({'source':'BaseLinker','day':payload['day'],'paid_today':payload['count'],'payment_histories_checked':b.checked_histories}),flush=True)
            if not args.scan_only:b.deliver(payload)
            b.first=False;retry=5
            if args.once:return
            time.sleep(max(1,config.get('poll_seconds',30)-(time.monotonic()-start)))
        except Exception as e:
            # Exception class / sanitized API code only. No raw responses or request headers.
            message=str(e) if isinstance(e,RuntimeError) else type(e).__name__
            print('bridge: '+message+'; retrying',flush=True)
            atomic(config['status_file'],{'connected':False,'checked_at':int(time.time()),'error':message})
            if args.once:raise SystemExit(1)
            time.sleep(retry);retry=min(60,retry*2)
if __name__=='__main__':main()
