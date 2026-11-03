/* Spatial 4x4 coverage rasterizer in native panel coordinates (pixel centres at integers). */
#ifndef TC002_RASTER_H
#define TC002_RASTER_H
#include <stdint.h>
#include <string.h>
#include <math.h>
#define RW 52
#define RH 16
#define RSS 4
static uint32_t raster_high[RW*RH*RSS*RSS],raster_low[RW*RH],raster_out[RW*RH];
static int raster_s=RSS;
static void raster_begin(int samples){raster_s=samples==1?1:RSS;memset(raster_high,0,sizeof(raster_high));memset(raster_low,0,sizeof(raster_low));}
static double raster_clamp(double n,double a,double b){return n<a?a:n>b?b:n;}
static int raster_start(double n,int lim){int v=(int)ceil((n+.5)*raster_s-.5);return v<0?0:v>=lim?lim:v;}
static int raster_end(double n,int lim){int v=(int)floor((n+.5)*raster_s-.5);return v<0?-1:v>=lim?lim-1:v;}
static void raster_rect(double x,double y,double w,double h,uint32_t c){
 if(w<=0||h<=0)return;int sw=RW*raster_s,sh=RH*raster_s,x0=raster_start(x-.5,sw),x1=raster_end(x+w-.5,sw),y0=raster_start(y-.5,sh),y1=raster_end(y+h-.5,sh);
 for(int j=y0;j<=y1;j++)for(int i=x0;i<=x1;i++){double px=(i+.5)/raster_s-.5,py=(j+.5)/raster_s-.5;if(px>=x-.5&&px<x+w-.5&&py>=y-.5&&py<y+h-.5)raster_high[j*sw+i]=c;}
 for(int j=(int)floor(y+.5);j<y+h;j++)for(int i=(int)floor(x+.5);i<x+w;i++)if(i>=0&&i<RW&&j>=0&&j<RH)raster_low[j*RW+i]=c;
}
static void raster_line(double x,double y,double xx,double yy,uint32_t c){
 int sw=RW*raster_s,sh=RH*raster_s,x0=raster_start(fmin(x,xx)-.5,sw),x1=raster_end(fmax(x,xx)+.5,sw),y0=raster_start(fmin(y,yy)-.5,sh),y1=raster_end(fmax(y,yy)+.5,sh);double dx=xx-x,dy=yy-y,len=dx*dx+dy*dy;
 for(int j=y0;j<=y1;j++)for(int i=x0;i<=x1;i++){double px=(i+.5)/raster_s-.5,py=(j+.5)/raster_s-.5,q=len?raster_clamp(((px-x)*dx+(py-y)*dy)/len,0,1):0,ex=px-x-q*dx,ey=py-y-q*dy;if(ex*ex+ey*ey<=.25)raster_high[j*sw+i]=c;}
}
static void raster_ellipse(double x,double y,double rx,double ry,uint32_t c,int fill){
 if(rx<=0||ry<=0)return;int sw=RW*raster_s,sh=RH*raster_s,x0=raster_start(x-rx-.5,sw),x1=raster_end(x+rx+.5,sw),y0=raster_start(y-ry-.5,sh),y1=raster_end(y+ry+.5,sh);double r=fmin(rx,ry);
 for(int j=y0;j<=y1;j++)for(int i=x0;i<=x1;i++){double dx=((i+.5)/raster_s-.5-x)/rx,dy=((j+.5)/raster_s-.5-y)/ry,v=sqrt(dx*dx+dy*dy);if(fill?v<=1:fabs(v-1)*r<=.5)raster_high[j*sw+i]=c;}
}
/* Procedural fields are sampled before reduction too: their contours need AA. */
static void raster_field(int kind,double t,uint32_t a,uint32_t b,uint32_t white){
 int sw=RW*raster_s,sh=RH*raster_s;double bx[3],by[3],co=cos(t*.45),si=sin(t*.45);
 for(int i=0;i<3;i++){bx[i]=26+sin(t*.7+i*2.1)*(12+i*2);by[i]=7.5+cos(t*.65+i*2.3)*4;}
 for(int j=0;j<sh;j++)for(int i=0;i<sw;i++){
  double x=(i+.5)/raster_s-.5,y=(j+.5)/raster_s-.5;uint32_t c=0;
  if(kind==0){double v=0;for(int k=0;k<3;k++){double dx=(x-bx[k])*.75,dy=y-by[k];v+=9/(dx*dx+dy*dy+1);}if(v>1.05&&v<1.8)c=a;else if(v>=1.8&&v<2.8)c=b;else if(v>4.8)c=white;}
  else if(y>=7.5){double z=10/(y-6),u=(x-26)*z*.12,v=z+t*.8;if(((int)floor(u*co-v*si)+(int)floor(u*si+v*co))%2==0)c=y>=11.5?a:b;}
  raster_high[j*sw+i]=c;
 }
}
static void raster_finish(void){int sw=RW*raster_s,area=raster_s*raster_s;for(int y=0;y<RH;y++)for(int x=0;x<RW;x++){unsigned r=0,g=0,b=0;for(int j=0;j<raster_s;j++)for(int i=0;i<raster_s;i++){uint32_t c=raster_high[(y*raster_s+j)*sw+x*raster_s+i];r+=c>>16&255;g+=c>>8&255;b+=c&255;}r=(r+area/2)/area;g=(g+area/2)/area;b=(b+area/2)/area;raster_out[y*RW+x]=(r<<16)|(g<<8)|b;}}
#endif
