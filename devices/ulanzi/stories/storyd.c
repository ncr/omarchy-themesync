/* TC002 local story engine. QuickJS MIT engine; scene source is shared with preview.
 * No firmware writes. Only aggregate counters and local display controls over authenticated LAN HTTP. */
#include "quickjs.h"
#include "raster.h"
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/input.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static double raster_arg(JSContext*c,JSValueConst *a,int i){double v=0;JS_ToFloat64(c,&v,a[i]);return v;}
static uint32_t raster_colour(JSContext*c,JSValueConst *a,int i){uint32_t v=0;JS_ToUint32(c,&v,a[i]);return v;}
static JSValue js_raster_begin(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;raster_begin(n?(int)raster_arg(c,a,0):4);return JS_UNDEFINED;}
static JSValue js_raster_rect(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;if(n==5)raster_rect(raster_arg(c,a,0),raster_arg(c,a,1),raster_arg(c,a,2),raster_arg(c,a,3),raster_colour(c,a,4));return JS_UNDEFINED;}
static JSValue js_raster_line(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;if(n==5)raster_line(raster_arg(c,a,0),raster_arg(c,a,1),raster_arg(c,a,2),raster_arg(c,a,3),raster_colour(c,a,4));return JS_UNDEFINED;}
static JSValue js_raster_ellipse(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;if(n==6)raster_ellipse(raster_arg(c,a,0),raster_arg(c,a,1),raster_arg(c,a,2),raster_arg(c,a,3),raster_colour(c,a,4),JS_ToBool(c,a[5]));return JS_UNDEFINED;}
static JSValue js_raster_field(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;if(n==5)raster_field((int)raster_arg(c,a,0),raster_arg(c,a,1),raster_colour(c,a,2),raster_colour(c,a,3),raster_colour(c,a,4));return JS_UNDEFINED;}
static JSValue js_raster_sample(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;if(n<2)return JS_UNDEFINED;int x=(int)floor(raster_arg(c,a,0)+.5),y=(int)floor(raster_arg(c,a,1)+.5);return JS_NewUint32(c,x>=0&&x<RW&&y>=0&&y<RH?raster_low[y*RW+x]:0);}
static JSValue js_raster_finish(JSContext*c,JSValueConst self,int n,JSValueConst*a){(void)self;(void)n;(void)a;raster_finish();JSValue buffer=JS_NewArrayBufferCopy(c,(uint8_t*)raster_out,sizeof(raster_out));if(JS_IsException(buffer))return buffer;JSValue args[]={buffer,JS_UNDEFINED,JS_UNDEFINED};JSValue result=JS_NewTypedArray(c,3,args,JS_TYPED_ARRAY_UINT32);JS_FreeValue(c,buffer);return result;}

static JSContext *ctx;
static char token[65], statepath[512];
static unsigned char lastframe[2496];
static int running=1, dry_run=0;
static unsigned long frames=0, failures=0;
static char last_api_status[80]="";
static double started,measured_fps=0,metrics_at=0,render_ms=0;
static unsigned long metrics_frames=0;
#define TARGET_FPS 60.0
static double mono(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec+t.tv_nsec/1e9;}
static void stop(int sig){(void)sig;running=0;}
static int writeall(int fd,const void *ptr,size_t n){const char *p=ptr;while(n){ssize_t k=send(fd,p,n,MSG_NOSIGNAL);if(k<0&&errno==EINTR)continue;if(k<=0)return -1;p+=k;n-=k;}return 0;}
static int socktime(int fd,int ms){struct timeval t={ms/1000,(ms%1000)*1000};setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&t,sizeof(t));setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&t,sizeof(t));return fd;}
static int api(const char *method,const char *path,const void *body,size_t len){
 if(dry_run)return 1;
 int fd=socktime(socket(AF_INET,SOCK_STREAM|SOCK_CLOEXEC,0),600);if(fd<0)return 0;
 int no_delay=1;setsockopt(fd,IPPROTO_TCP,TCP_NODELAY,&no_delay,sizeof(no_delay));
 struct sockaddr_in a={.sin_family=AF_INET,.sin_port=htons(80)};inet_pton(AF_INET,"127.0.0.1",&a.sin_addr);
 if(connect(fd,(void*)&a,sizeof(a))){close(fd);return 0;}
 char h[1024];int n=snprintf(h,sizeof(h),"%s /api/v1%s HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer %s\r\nContent-Type: %s\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",method,path,token,!strncmp(path,"/frame?",7)?"application/octet-stream":"application/json",len);
 int ok=writeall(fd,h,n)==0&&(!len||writeall(fd,body,len)==0);char response[2048];int k=ok?recv(fd,response,sizeof(response)-1,0):-1;if(k>0)response[k]=0;
 ok=k>12&&!strncmp(response,"HTTP/1.1 200",12);
 if(k>0){size_t z=strcspn(response,"\r\n");if(z>79)z=79;memcpy(last_api_status,response,z);last_api_status[z]=0;}
 if(!ok&&failures<4&&k>0)fprintf(stderr,"API response: %.*s\n",k,response);
 close(fd);return ok;
}
static void error(void){JSValue e=JS_GetException(ctx);const char*s=JS_ToCString(ctx,e);fprintf(stderr,"story JS: %s\n",s?s:"exception");JS_FreeCString(ctx,s);JSValue stack=JS_GetPropertyStr(ctx,e,"stack");s=JS_ToCString(ctx,stack);if(s)fprintf(stderr,"%s\n",s);JS_FreeCString(ctx,s);JS_FreeValue(ctx,stack);JS_FreeValue(ctx,e);}
static JSValue call(const char *name,int n,JSValue *args){JSValue global=JS_GetGlobalObject(ctx),f=JS_GetPropertyStr(ctx,global,name),v=JS_Call(ctx,f,global,n,args);JS_FreeValue(ctx,f);JS_FreeValue(ctx,global);return v;}
static JSValue js_time(JSContext*c,JSValueConst self,int n,JSValueConst*args){(void)self;(void)n;(void)args;return JS_NewFloat64(c,mono()-started);}
static JSValue js_day(JSContext*c,JSValueConst self,int n,JSValueConst*args){(void)self;(void)n;(void)args;time_t now=time(NULL);struct tm t;char b[16];localtime_r(&now,&t);strftime(b,sizeof(b),"%Y-%m-%d",&t);return JS_NewString(c,b);}
static JSValue js_save(JSContext*c,JSValueConst self,int n,JSValueConst*args){(void)self;if(n!=1)return JS_FALSE;size_t len;const char*s=JS_ToCStringLen(c,&len,args[0]);if(!s||len>8192){JS_FreeCString(c,s);return JS_FALSE;}char tmp[550];snprintf(tmp,sizeof(tmp),"%s.new",statepath);FILE*f=fopen(tmp,"w");int ok=0;if(f){chmod(tmp,0600);ok=fwrite(s,1,len,f)==len&&fflush(f)==0;int syncok=fsync(fileno(f))==0;int closeok=fclose(f)==0;ok=ok&&syncok&&closeok;if(ok)ok=rename(tmp,statepath)==0;}JS_FreeCString(c,s);return JS_NewBool(c,ok);}
static JSValue js_load(JSContext*c,JSValueConst self,int n,JSValueConst*args){(void)self;(void)n;(void)args;char b[8193];FILE*f=fopen(statepath,"r");if(!f)return JS_NULL;size_t k=fread(b,1,8192,f);b[k]=0;fclose(f);return JS_ParseJSON(c,b,k,"state.json");}
static JSValue js_api(JSContext*c,JSValueConst self,int n,JSValueConst*args){(void)self;if(n!=3)return JS_FALSE;const char*m=JS_ToCString(c,args[0]),*p=JS_ToCString(c,args[1]),*b=JS_ToCString(c,args[2]);int ok=m&&p&&b&&api(m,p,b,strlen(b));JS_FreeCString(c,m);JS_FreeCString(c,p);JS_FreeCString(c,b);return JS_NewBool(c,ok);}
static JSValue js_info(JSContext*c,JSValueConst self,int n,JSValueConst*args){(void)self;(void)n;(void)args;JSValue v=JS_NewObject(c);JS_SetPropertyStr(c,v,"target_fps",JS_NewFloat64(c,TARGET_FPS));JS_SetPropertyStr(c,v,"fps",JS_NewFloat64(c,measured_fps));JS_SetPropertyStr(c,v,"render_ms",JS_NewFloat64(c,render_ms));JS_SetPropertyStr(c,v,"antialias_samples",JS_NewInt32(c,16));JS_SetPropertyStr(c,v,"frames",JS_NewInt64(c,frames));JS_SetPropertyStr(c,v,"last_api_status",JS_NewString(c,last_api_status));JS_SetPropertyStr(c,v,"frame_failures",JS_NewInt64(c,failures));JS_SetPropertyStr(c,v,"uptime_s",JS_NewFloat64(c,mono()-started));return v;}
static int evalfile(const char *path){FILE*f=fopen(path,"r");if(!f){perror(path);return 0;}fseek(f,0,SEEK_END);long n=ftell(f);rewind(f);if(n<0||n>200000){fclose(f);return 0;}char*b=malloc(n+1);if(!b){fclose(f);return 0;}size_t k=fread(b,1,n,f);fclose(f);b[k]=0;JSValue v=JS_Eval(ctx,b,k,path,JS_EVAL_TYPE_GLOBAL);free(b);int ok=!JS_IsException(v);if(!ok)error();JS_FreeValue(ctx,v);return ok;}
static void control(const char *name,const char *edge){JSValue a[]={JS_NewString(ctx,name),JS_NewString(ctx,edge)};JSValue v=call("storyControl",2,a);if(JS_IsException(v))error();for(int i=0;i<2;i++)JS_FreeValue(ctx,a[i]);JS_FreeValue(ctx,v);}
struct TcInputEvent { int32_t sec,usec; uint16_t type,code; int32_t value; };
static void input(int fd,int knob){static int half=0;static double down[4]={0};static int held[4]={0};struct TcInputEvent ev;
 while(read(fd,&ev,sizeof(ev))==sizeof(ev)){
  if(knob&&ev.type==EV_ABS){int prev=half;half=ev.value==8?1:ev.value==13?-1:0;if(ev.value==1&&prev==1)control("rotary","cw");if(ev.value==11&&prev==-1)control("rotary","ccw");}
  if(!knob&&ev.type==EV_KEY){int idx=ev.code==108?0:ev.code==105?1:ev.code==106?2:ev.code==103?3:-1;if(idx<0)continue;const char *names[]={"left","middle","right","knob"};if(ev.value==1){down[idx]=mono();held[idx]=1;}if(ev.value==0&&held[idx]){held[idx]=0;control(names[idx],mono()-down[idx]>.8?"long":"click");}}
 }
}
static void reply(int fd,int code,const char*type,const void*b,size_t n){char h[256];int k=snprintf(h,sizeof(h),"HTTP/1.1 %d %s\r\nContent-Type: %s\r\nContent-Length: %zu\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",code,code==200?"OK":"Error",type,n);if(writeall(fd,h,k)==0)writeall(fd,b,n);}
/* One bounded request, no keep-alive/chunks, 200 ms deadline. No browser CORS. */
static void serve(int listener){int fd=accept4(listener,NULL,NULL,SOCK_CLOEXEC);if(fd<0)return;socktime(fd,200);char buf[12289],method[12],path[128];size_t used=0,head=0,len=0;double deadline=mono()+.25;
 while(used<sizeof(buf)-1&&mono()<deadline){int n=recv(fd,buf+used,sizeof(buf)-1-used,0);if(n<=0)break;used+=n;buf[used]=0;char *e=strstr(buf,"\r\n\r\n");if(e){head=e-buf+4;char *s=strcasestr(buf,"\r\nContent-Length:");if(s&&s<e){char *end;unsigned long z=strtoul(s+17,&end,10);if(z>8192||end==s+17){reply(fd,413,"application/json","{}",2);goto done;}len=z;}if(used>=head+len)break;}}
 if(!head||used!=head+len||sscanf(buf,"%11s %127s",method,path)!=2){reply(fd,400,"application/json","{}",2);goto done;}
 char *auth=strcasestr(buf,"\r\nauthorization:");int authenticated=0;
 if(auth&&auth<buf+head){char *v=auth+16;while(v<buf+head&&(*v==' '||*v=='\t'))v++;if(v+73<=buf+head&&!strncasecmp(v,"Bearer ",7)&&!memcmp(v+7,token,64)&&v[71]=='\r'&&v[72]=='\n')authenticated=1;}
 if(!authenticated){reply(fd,401,"application/json","{}",2);goto done;}
 if(!strcmp(method,"GET")&&!strcmp(path,"/frame")){reply(fd,200,"application/octet-stream",lastframe,sizeof(lastframe));goto done;}
 JSValue body=len?JS_ParseJSON(ctx,buf+head,len,"request"):JS_NewObject(ctx);if(JS_IsException(body)){JS_FreeValue(ctx,JS_GetException(ctx));reply(fd,400,"application/json","{}",2);goto done;}
 JSValue args[]={JS_NewString(ctx,method),JS_NewString(ctx,path),body};JSValue v=call("storyRequest",3,args);for(int i=0;i<3;i++)JS_FreeValue(ctx,args[i]);if(JS_IsException(v)){error();reply(fd,400,"application/json","{\"error\":\"invalid_request\"}",27);}else{JSValue j=JS_JSONStringify(ctx,v,JS_UNDEFINED,JS_UNDEFINED);size_t n;const char*s=JS_ToCStringLen(ctx,&n,j);if(s)reply(fd,200,"application/json",s,n);else reply(fd,500,"application/json","{}",2);JS_FreeCString(ctx,s);JS_FreeValue(ctx,j);}JS_FreeValue(ctx,v);
 done:close(fd);
}
int main(int argc,char**argv){
 if(argc<5){fprintf(stderr,"usage: storyd renderer.js device.js tokens state.json [port] [--dry-run]\n");return 2;}
 setenv("TZ","CET-1CEST,M3.5.0,M10.5.0/3",1);tzset();umask(0077);signal(SIGTERM,stop);signal(SIGINT,stop);signal(SIGPIPE,SIG_IGN);started=mono();
 snprintf(statepath,sizeof(statepath),"%s",argv[4]);char lock[550];snprintf(lock,sizeof(lock),"%s.lock",statepath);int lockfd=open(lock,O_CREAT|O_RDWR|O_CLOEXEC,0600);if(lockfd<0||flock(lockfd,LOCK_EX|LOCK_NB)){fprintf(stderr,"storyd already running or state path unavailable\n");return 1;}
 FILE*f=fopen(argv[3],"r");if(!f){perror("tokens");return 1;}char line[256];while(fgets(line,sizeof(line),f))if(!strncmp(line,"control=",8)&&strlen(line+8)>=64){memcpy(token,line+8,64);token[64]=0;}fclose(f);if(strlen(token)!=64){fprintf(stderr,"missing control token\n");return 1;}
 dry_run=argc>6&&!strcmp(argv[6],"--dry-run");int port=argc>5?atoi(argv[5]):8878;if(port<1024||port>65535)return 2;
 JSRuntime *rt=JS_NewRuntime();JS_SetMemoryLimit(rt,3*1024*1024);JS_SetMaxStackSize(rt,512*1024);ctx=JS_NewContext(rt);if(!ctx)return 1;
 JSValue g=JS_GetGlobalObject(ctx);JS_SetPropertyStr(ctx,g,"nativeRasterBegin",JS_NewCFunction(ctx,js_raster_begin,"nativeRasterBegin",1));JS_SetPropertyStr(ctx,g,"nativeRasterRect",JS_NewCFunction(ctx,js_raster_rect,"nativeRasterRect",5));JS_SetPropertyStr(ctx,g,"nativeRasterLine",JS_NewCFunction(ctx,js_raster_line,"nativeRasterLine",5));JS_SetPropertyStr(ctx,g,"nativeRasterEllipse",JS_NewCFunction(ctx,js_raster_ellipse,"nativeRasterEllipse",6));JS_SetPropertyStr(ctx,g,"nativeRasterField",JS_NewCFunction(ctx,js_raster_field,"nativeRasterField",5));JS_SetPropertyStr(ctx,g,"nativeRasterSample",JS_NewCFunction(ctx,js_raster_sample,"nativeRasterSample",2));JS_SetPropertyStr(ctx,g,"nativeRasterFinish",JS_NewCFunction(ctx,js_raster_finish,"nativeRasterFinish",0));JS_SetPropertyStr(ctx,g,"nativeTime",JS_NewCFunction(ctx,js_time,"nativeTime",0));JS_SetPropertyStr(ctx,g,"nativeDay",JS_NewCFunction(ctx,js_day,"nativeDay",0));JS_SetPropertyStr(ctx,g,"nativeSave",JS_NewCFunction(ctx,js_save,"nativeSave",1));JS_SetPropertyStr(ctx,g,"nativeLoad",JS_NewCFunction(ctx,js_load,"nativeLoad",0));JS_SetPropertyStr(ctx,g,"nativeApi",JS_NewCFunction(ctx,js_api,"nativeApi",3));JS_SetPropertyStr(ctx,g,"nativeInfo",JS_NewCFunction(ctx,js_info,"nativeInfo",0));JS_FreeValue(ctx,g);
 if(!evalfile(argv[1]))return 1;
 char demos_path[1024];snprintf(demos_path,sizeof(demos_path),"%s",argv[1]);char *slash=strrchr(demos_path,'/');
 if(slash)snprintf(slash+1,sizeof(demos_path)-(slash+1-demos_path),"demos.js");else snprintf(demos_path,sizeof(demos_path),"demos.js");
 if(access(demos_path,R_OK)==0&&!evalfile(demos_path))return 1;
 if(!evalfile(argv[2]))return 1;
 int listener=socket(AF_INET,SOCK_STREAM|SOCK_CLOEXEC,0),yes=1;setsockopt(listener,SOL_SOCKET,SO_REUSEADDR,&yes,sizeof(yes));struct sockaddr_in a={.sin_family=AF_INET,.sin_port=htons(port),.sin_addr.s_addr=INADDR_ANY};if(bind(listener,(void*)&a,sizeof(a))||listen(listener,4)){perror("story HTTP");return 1;}fcntl(listener,F_SETFL,O_NONBLOCK);
 int keys=open("/dev/input/event67",O_RDONLY|O_NONBLOCK|O_CLOEXEC),knob=open("/dev/input/event68",O_RDONLY|O_NONBLOCK|O_CLOEXEC);fprintf(stderr,"storyd ready; local 52x16 scenes, port %d; keys=%d knob=%d\n",port,keys>=0,knob>=0);
 double next=mono();metrics_at=next;while(running){
  struct pollfd p[]={{listener,POLLIN,0},{keys,POLLIN,0},{knob,POLLIN,0}};int wait=(int)ceil((next-mono())*1000);poll(p,3,wait<0?0:wait>17?17:wait);
  if(p[0].revents&POLLIN)serve(listener);if(p[1].revents&POLLIN)input(keys,0);if(p[2].revents&POLLIN)input(knob,1);
  if(mono()>=next){double frame_start=mono();JSValue v=call("storyTick",0,NULL);if(JS_IsException(v)){error();running=0;}else if(!JS_IsNull(v)&&!JS_IsUndefined(v)){size_t off=0,n=0,elem=0,size=0;JSValue b=JS_GetTypedArrayBuffer(ctx,v,&off,&n,&elem);uint8_t*data=JS_GetArrayBuffer(ctx,&size,b);if(data&&n==2496&&off+n<=size){memcpy(lastframe,data+off,2496);if(api("POST","/frame?duration_s=3&transition=cut&stream=1",data+off,n))frames++;else failures++;}else{fprintf(stderr,"invalid story frame\n");running=0;}JS_FreeValue(ctx,b);}JS_FreeValue(ctx,v);double end=mono();render_ms=render_ms*.9+(end-frame_start)*1000*.1;next+=1.0/TARGET_FPS;if(next<end)next=end;
   if(end-metrics_at>=1){measured_fps=(frames-metrics_frames)/(end-metrics_at);metrics_frames=frames;metrics_at=end;}}
 }
 close(listener);if(keys>=0)close(keys);if(knob>=0)close(knob);JS_FreeContext(ctx);JS_FreeRuntime(rt);close(lockfd);return 0;
}
