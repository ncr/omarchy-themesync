#!/usr/bin/env python3
"""Arrival sounds for the five order scenes, synthesised (no samples) and reproducible.

All five sit in one key (A major pentatonic), are short and gentle, and line up with the moment
they belong to in the animation (renderer.js `cue`). 16 kHz mono 16-bit (within the 256 KiB the clock keeps for sounds), as the TC002 plays them.

  python3 make_sounds.py OUT_DIR
"""
import sys,wave
from pathlib import Path
import numpy as np
RATE=16000
A5,CS6,E6,FS6,A6,CS7,E7=880.0,1108.73,1318.51,1479.98,1760.0,2217.46,2637.02
rng=np.random.default_rng(20261105)

def t(seconds):return np.arange(int(RATE*seconds))/RATE
def place(out,at,sound):
    i=int(at*RATE);out[i:i+len(sound)]+=sound[:max(0,len(out)-i)]
def env(n_attack,decay,length):
    x=t(length);a=np.clip(x/max(n_attack,1e-4),0,1);return a*np.exp(-x/decay)
def partials(freqs,amps,decays,length,attack=.002):
    x=t(length);return sum(a*np.sin(2*np.pi*f*x)*np.clip(x/attack,0,1)*np.exp(-x/d) for f,a,d in zip(freqs,amps,decays))
def resonant_noise(centre,width,length,decay):
    # A noise burst through a narrow band around `centre`: the body of a knock.
    n=rng.standard_normal(int(RATE*length));spec=np.fft.rfft(n);f=np.fft.rfftfreq(len(n),1/RATE)
    spec*=np.exp(-((f-centre)/width)**2);return np.fft.irfft(spec,len(n))/3*np.exp(-t(length)/decay)

def bead():
    """Liczydło: a wooden bead lands on its stack, then settles with a softer tap."""
    def knock(level):return level*(resonant_noise(1700,500,.09,.012)+.55*partials([E6/2,A5],[.5,.25],[.03,.02],.09))
    out=np.zeros(int(RATE*.6));place(out,0,knock(1));place(out,.25,knock(.4));return out

def star():
    """Gwiazdozbiór: a rising comet hiss, then a small glass bell as the star lights."""
    out=np.zeros(int(RATE*2.2));x=t(1.35)
    hiss=rng.standard_normal(len(x));spec=np.fft.rfft(hiss);f=np.fft.rfftfreq(len(x),1/RATE);spec*=np.exp(-((f-5000)/2500)**2)
    hiss=np.fft.irfft(spec,len(x));hiss*=(x/1.35)**2*np.clip((1.35-x)/.06,0,1)*.25;place(out,0,hiss)
    place(out,1.35,partials([E7,E7*2.76/2,CS7*1.5],[.55,.22,.12],[.6,.3,.18],.85,.001))
    place(out,1.47,.35*partials([A6],[1],[.45],.73,.001));return out

def marble():
    """Słoik: a glass marble clinks on the pile, bounces twice and rests."""
    def clink(level):return level*partials([A6*1.27,CS7*1.53,E7*1.81],[.6,.3,.18],[.07,.04,.025],.25,.0005)
    out=np.zeros(int(RATE*.9));[place(out,at,clink(l)) for at,l in [(0,1),(.19,.55),(.32,.3),(.41,.15)]];return out

def flaps():
    """Tablica: split-flap clatter, one click per character as it turns over (times match the flaps)."""
    def click(level):return level*(resonant_noise(3200,1400,.03,.004)+.3*resonant_noise(900,300,.03,.006))
    times=[.30,.42,.50,.57,.71,.70,.77,.78,.91,.98]
    out=np.zeros(int(RATE*1.2));[place(out,at,click(.9-.04*i+.1*rng.random())) for i,at in enumerate(sorted(times))];return out

def chime():
    """Witraż: a glass-harmonica chord that swells as the pane grows in."""
    x=t(2.2);shape=np.clip(x/.6,0,1)**2*np.exp(-np.clip(x-.8,0,None)/.5);wobble=1+.003*np.sin(2*np.pi*5*x)
    out=sum(a*np.sin(2*np.pi*f*np.cumsum(wobble)/RATE) for f,a in [(A5,.4),(CS6,.3),(E6,.28),(A6,.12)])*shape
    return out*np.clip((2.2-x)/.2,0,1)

SOUNDS={'order-bead':bead,'order-star':star,'order-marble':marble,'order-flaps':flaps,'order-chime':chime}

def write(path,signal):
    peak=np.max(np.abs(signal)) or 1;pcm=(signal/peak*.8*32767).astype('<i2')
    with wave.open(str(path),'wb') as w:w.setnchannels(1);w.setsampwidth(2);w.setframerate(RATE);w.writeframes(pcm.tobytes())

def main():
    out=Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=True)
    for name,make in SOUNDS.items():write(out/(name+'.wav'),make());print(name,(out/(name+'.wav')).stat().st_size,'bytes')
if __name__=='__main__':main()
