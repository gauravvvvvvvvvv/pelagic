#ifndef PELAGIC_H
#define PELAGIC_H
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { int32_t valid; double time,x,y,z,vx,vy,vz; } PelagicState;
typedef struct { int32_t valid; double height,bottom,ux,uy,uz,density,sound_speed; } PelagicSample;
uint32_t pelagic_abi_version(void);
uint64_t pelagic_create(uint64_t seed);
/* UTF-8 JSON, readable for length bytes; returns 0 on invalid input/capacity. */
uint64_t pelagic_create_scene(const uint8_t *bytes, size_t length);
int32_t pelagic_destroy(uint64_t handle);
int32_t pelagic_step(uint64_t handle,uint32_t ticks);
int32_t pelagic_command(uint64_t handle,uint64_t entity,double fx,double fy,double fz,double tx,double ty,double tz);
PelagicState pelagic_state(uint64_t handle,uint64_t entity);
PelagicSample pelagic_sample(uint64_t handle,double x,double y,double z,double time);
#ifdef __cplusplus
}
#endif
#endif
