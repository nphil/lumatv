// Hold N 1080p DRM dumb buffers (contiguous CMA on lg1k) for S seconds, logging CmaFree, to check 4K HDR decode is not starved.
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <time.h>
struct drm_mode_create_dumb { uint32_t height, width, bpp, flags, handle, pitch; uint64_t size; };
struct drm_gem_close { uint32_t handle, pad; };
#define DRM_IOCTL_MODE_CREATE_DUMB _IOWR('d', 0xB2, struct drm_mode_create_dumb)
#define DRM_IOCTL_GEM_CLOSE _IOW('d', 0x09, struct drm_gem_close)
static long mi(const char *k){FILE*f=fopen("/proc/meminfo","r");char l[256];long v=-1;while(fgets(l,sizeof l,f))if(!strncmp(l,k,strlen(k))){sscanf(l+strlen(k)," %ld",&v);break;}fclose(f);return v;}
int main(int c,char**v){int n=c>1?atoi(v[1]):3,s=c>2?atoi(v[2]):40;int fd=open("/dev/dri/card0",O_RDWR|O_CLOEXEC);uint32_t h[16];int got=0;
 printf("before CmaFree %ld kB\n",mi("CmaFree:"));
 for(;got<n;got++){struct timespec a,b;clock_gettime(CLOCK_MONOTONIC,&a);struct drm_mode_create_dumb d={1080,1920,32,0,0,0,0};
  if(ioctl(fd,DRM_IOCTL_MODE_CREATE_DUMB,&d)<0){perror("create_dumb");break;}clock_gettime(CLOCK_MONOTONIC,&b);
  h[got]=d.handle;printf("buf %d ok in %.1f ms, CmaFree %ld kB\n",got+1,(b.tv_sec-a.tv_sec)*1e3+(b.tv_nsec-a.tv_nsec)/1e6,mi("CmaFree:"));}
 for(int t=0;t<s;t+=10){sleep(10);printf("t+%d s CmaFree %ld kB\n",t+10,mi("CmaFree:"));fflush(stdout);}
 for(int i=0;i<got;i++){struct drm_gem_close g={h[i],0};ioctl(fd,DRM_IOCTL_GEM_CLOSE,&g);}
 printf("freed CmaFree %ld kB\n",mi("CmaFree:"));return 0;}
