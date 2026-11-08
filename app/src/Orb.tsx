import { useEffect, useRef, useState } from "react";

/** The jitmo orb: a glowing ring of swirling colour that breathes and trails a
 *  light around it. It is drawn by a small shader written for this app (raw
 *  WebGL, no library). If WebGL is not available it falls back to a plain CSS
 *  orb, so the bar always has a mark. It runs faster while a request works and
 *  reacts to the pointer. */

const VERT = `
attribute vec2 p;
void main() { gl_Position = vec4(p, 0.0, 1.0); }
`;

const FRAG = `
precision mediump float;
uniform vec2 uRes;
uniform float uTime;
uniform float uBoost;

float hash(vec2 q) {
  return fract(sin(dot(q, vec2(127.1, 311.7))) * 43758.5453);
}
float vnoise(vec2 q) {
  vec2 i = floor(q);
  vec2 f = fract(q);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
             mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

void main() {
  vec2 p = (gl_FragCoord.xy * 2.0 - uRes) / min(uRes.x, uRes.y);
  float r = length(p);
  float a = atan(p.y, p.x);
  float t = uTime;

  // Noise on the unit circle, so the swirl has no seam.
  vec2 ring = vec2(cos(a), sin(a));
  float n = vnoise(ring * 1.5 + vec2(t * 0.35, -t * 0.27));
  float n2 = vnoise(ring * 2.6 + vec2(-t * 0.5, t * 0.4));

  float R = 0.66 + 0.035 * sin(t * 1.3) + (n - 0.5) * 0.07;
  float d = abs(r - R);

  float band = smoothstep(0.24, 0.0, d);
  float core = smoothstep(0.07, 0.0, d);
  float halo = exp(-d * 4.2) * 0.42 * uBoost;

  // Colours drift around the ring: cyan, blue, violet, pink.
  vec3 cyan = vec3(0.30, 0.90, 1.00);
  vec3 blue = vec3(0.25, 0.42, 1.00);
  vec3 violet = vec3(0.66, 0.30, 1.00);
  vec3 pink = vec3(1.00, 0.42, 0.86);
  float w1 = 0.5 + 0.5 * sin(a + t * 0.7 + n * 3.0);
  float w2 = 0.5 + 0.5 * sin(a * 2.0 - t * 0.5 + n2 * 3.0);
  float w3 = 0.5 + 0.5 * sin(a - t * 0.3 + 1.7);
  vec3 col = mix(mix(cyan, violet, w1), mix(blue, pink, w2), w3);

  // A bright light that orbits the ring.
  float la = t * 1.15;
  float da = abs(mod(a - la + 3.14159, 6.28318) - 3.14159);
  float spot = exp(-da * da * 5.0) * smoothstep(0.22, 0.0, d);

  vec3 c = col * band * 1.1 + vec3(1.0) * core * 0.6 + vec3(1.0, 0.96, 1.0) * spot * 0.85 + col * halo;
  float alpha = clamp(max(max(band * 0.95, halo), spot), 0.0, 1.0);
  gl_FragColor = vec4(c * alpha, alpha);
}
`;

function compile(gl: WebGLRenderingContext, type: number, src: string) {
  const s = gl.createShader(type);
  if (!s) return null;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
}

export default function Orb({ working = false, size = 34 }: { working?: boolean; size?: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const boost = useRef(1);
  const speed = useRef(1);
  const [fallback, setFallback] = useState(false);

  // What the shader should do right now.
  useEffect(() => {
    speed.current = working ? 3 : 1;
  }, [working]);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const gl = canvas.getContext("webgl", { alpha: true, premultipliedAlpha: true, antialias: true });
    if (!gl) {
      setFallback(true);
      return;
    }
    const vs = compile(gl, gl.VERTEX_SHADER, VERT);
    const fs = compile(gl, gl.FRAGMENT_SHADER, FRAG);
    const prog = gl.createProgram();
    if (!vs || !fs || !prog) {
      setFallback(true);
      return;
    }
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      setFallback(true);
      return;
    }
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, "p");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const uRes = gl.getUniformLocation(prog, "uRes");
    const uTime = gl.getUniformLocation(prog, "uTime");
    const uBoost = gl.getUniformLocation(prog, "uBoost");

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(size * dpr);
    canvas.height = Math.round(size * dpr);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.uniform2f(uRes, canvas.width, canvas.height);
    gl.clearColor(0, 0, 0, 0);

    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let raf = 0;
    let t = Math.random() * 20;
    let last = performance.now();
    const frame = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      t += dt * speed.current * (reduce ? 0.15 : 0.55);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.uniform1f(uTime, t);
      gl.uniform1f(uBoost, boost.current);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    const lost = (e: Event) => {
      e.preventDefault();
      cancelAnimationFrame(raf);
      setFallback(true);
    };
    canvas.addEventListener("webglcontextlost", lost);
    return () => {
      cancelAnimationFrame(raf);
      canvas.removeEventListener("webglcontextlost", lost);
    };
  }, [size]);

  if (fallback) {
    return <span className={`orb${working ? " working" : ""}`} data-tauri-drag-region aria-hidden="true" />;
  }
  return (
    <canvas
      ref={ref}
      className="orb-canvas"
      style={{ width: size, height: size }}
      data-tauri-drag-region
      aria-hidden="true"
      onMouseEnter={() => (boost.current = 1.6)}
      onMouseLeave={() => (boost.current = 1)}
    />
  );
}
