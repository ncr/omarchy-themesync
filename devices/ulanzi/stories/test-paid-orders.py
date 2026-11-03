import importlib.util,unittest,tempfile,datetime as dt
from pathlib import Path
from unittest.mock import patch
s=importlib.util.spec_from_file_location('paid_orders',Path(__file__).with_name('paid_orders.py'));p=importlib.util.module_from_spec(s);s.loader.exec_module(p)
class Tests(unittest.TestCase):
 def test_full_payment_date_not_creation_and_partials(self):
  history=[{'paid_before':0,'paid_after':30,'total_price':100,'date':1},{'paid_before':30,'paid_after':100,'total_price':100,'date':2},{'paid_before':100,'paid_after':110,'total_price':100,'date':3}]
  self.assertEqual(p.first_full_payment(history),2)
  self.assertIsNone(p.first_full_payment(history[:1]))
  self.assertIsNone(p.first_full_payment([{'paid_before':100,'paid_after':100,'total_price':90,'date':4}]))
 def test_decimal_total_and_no_pii(self):
  v=p.summary({'order_id':1,'email':'private@example.invalid','delivery_price':'0.10','payment_done':'0.30','products':[{'price_brutto':'0.1','quantity':2}]})
  self.assertTrue(p.fully_paid(v));self.assertNotIn('email',v);self.assertEqual(v['total'],'0.30')
 def test_dst_days(self):
  for day,hours in [('2026-03-29',23),('2026-10-25',25)]:
   now=dt.datetime.fromisoformat(day+'T12:00').replace(tzinfo=p.TZ).timestamp();_,a,b=p.day_window(now);self.assertEqual(b-a,hours*3600)
 def test_paid_history_delay_retry_and_restart_dedup(self):
  class API:
   calls=0
   def call(self,*args):
    self.calls+=1
    return {'payments':[] if self.calls==1 else [{'paid_before':'0','paid_after':'10','total_price':'10','date':100}]}
  with tempfile.TemporaryDirectory() as tmp:
   conf={'state_file':str(Path(tmp)/'state.json')};api=API();b=p.Bridge(conf,api);b.state['day']='x'
   o={'id':1,'created':5,'paid':'10','total':'10'}
   b.consider(o,50,150);self.assertEqual(len(b.state['paid']),0)
   b.consider(o,50,150);self.assertEqual(len(b.state['paid']),1)
   b.consider(o,50,150);self.assertEqual(api.calls,2)
   p.atomic(conf['state_file'],b.state);r=p.Bridge(conf,api);r.consider(o,50,150);self.assertEqual(len(r.state['paid']),1);self.assertEqual(api.calls,2)
 def test_previous_day_already_full_does_not_count_topup_today(self):
  h=[{'paid_before':0,'paid_after':10,'total_price':10,'date':1},{'paid_before':10,'paid_after':20,'total_price':20,'date':101}]
  self.assertEqual(p.first_full_payment(h),1)
if __name__=='__main__':unittest.main()
