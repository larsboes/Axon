import type { FluidConfig } from './types';

const DEFAULT_CFG: FluidConfig = {
  simRes: 128,
  dyeRes: 640,
  pressureIters: 18,
  curl: 26,
  velDiss: 0.22,
  dyeDiss: 0.9,
  splatForce: 5200,
  maxDpr: 1.25,
};

const VERT = `#version 300 es
precision highp float;
in vec2 aPosition;
out vec2 vUv, vL, vR, vT, vB;
uniform vec2 texelSize;

void main () {
  vUv = aPosition * 0.5 + 0.5;
  vL = vUv - vec2(texelSize.x, 0.0);
  vR = vUv + vec2(texelSize.x, 0.0);
  vT = vUv + vec2(0.0, texelSize.y);
  vB = vUv - vec2(0.0, texelSize.y);
  gl_Position = vec4(aPosition, 0.0, 1.0);
}`;

const FRAG = {
  clear: `#version 300 es
precision mediump float;
in vec2 vUv;
out vec4 fragColor;
uniform sampler2D uTexture;
uniform float value;
void main () { fragColor = value * texture(uTexture, vUv); }`,

  splat: `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 fragColor;
uniform sampler2D uTarget;
uniform float aspectRatio;
uniform vec3 color;
uniform vec2 point;
uniform float radius;
void main () {
  vec2 p = vUv - point.xy; p.x *= aspectRatio;
  vec3 splat = exp(-dot(p, p) / radius) * color;
  vec3 base = texture(uTarget, vUv).xyz;
  fragColor = vec4(base + splat, 1.0);
}`,

  advection: `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 fragColor;
uniform sampler2D uVelocity;
uniform sampler2D uSource;
uniform vec2 texelSize;
uniform float dt;
uniform float dissipation;
void main () {
  vec2 coord = vUv - dt * texture(uVelocity, vUv).xy * texelSize;
  vec4 result = texture(uSource, coord);
  fragColor = result / (1.0 + dissipation * dt);
}`,

  divergence: `#version 300 es
precision mediump float;
in vec2 vUv, vL, vR, vT, vB;
out vec4 fragColor;
uniform sampler2D uVelocity;
void main () {
  float L = texture(uVelocity, vL).x;
  float R = texture(uVelocity, vR).x;
  float T = texture(uVelocity, vT).y;
  float B = texture(uVelocity, vB).y;
  vec2 C = texture(uVelocity, vUv).xy;
  if (vL.x < 0.0) { L = -C.x; }
  if (vR.x > 1.0) { R = -C.x; }
  if (vT.y > 1.0) { T = -C.y; }
  if (vB.y < 0.0) { B = -C.y; }
  fragColor = vec4(0.5 * (R - L + T - B), 0.0, 0.0, 1.0);
}`,

  curl: `#version 300 es
precision mediump float;
in vec2 vUv, vL, vR, vT, vB;
out vec4 fragColor;
uniform sampler2D uVelocity;
void main () {
  float L = texture(uVelocity, vL).y;
  float R = texture(uVelocity, vR).y;
  float T = texture(uVelocity, vT).x;
  float B = texture(uVelocity, vB).x;
  fragColor = vec4(0.5 * (R - L - T + B), 0.0, 0.0, 1.0);
}`,

  vorticity: `#version 300 es
precision highp float;
in vec2 vUv, vL, vR, vT, vB;
out vec4 fragColor;
uniform sampler2D uVelocity;
uniform sampler2D uCurl;
uniform float curl;
uniform float dt;
void main () {
  float L = texture(uCurl, vL).x;
  float R = texture(uCurl, vR).x;
  float T = texture(uCurl, vT).x;
  float B = texture(uCurl, vB).x;
  float C = texture(uCurl, vUv).x;
  vec2 force = 0.5 * vec2(abs(T) - abs(B), abs(R) - abs(L));
  force /= length(force) + 0.0001;
  force *= curl * C; force.y *= -1.0;
  vec2 velocity = texture(uVelocity, vUv).xy + force * dt;
  velocity = clamp(velocity, -1000.0, 1000.0);
  fragColor = vec4(velocity, 0.0, 1.0);
}`,

  pressure: `#version 300 es
precision mediump float;
in vec2 vUv, vL, vR, vT, vB;
out vec4 fragColor;
uniform sampler2D uPressure;
uniform sampler2D uDivergence;
void main () {
  float L = texture(uPressure, vL).x;
  float R = texture(uPressure, vR).x;
  float T = texture(uPressure, vT).x;
  float B = texture(uPressure, vB).x;
  float divergence = texture(uDivergence, vUv).x;
  fragColor = vec4((L + R + B + T - divergence) * 0.25, 0.0, 0.0, 1.0);
}`,

  gradient: `#version 300 es
precision mediump float;
in vec2 vUv, vL, vR, vT, vB;
out vec4 fragColor;
uniform sampler2D uPressure;
uniform sampler2D uVelocity;
void main () {
  float L = texture(uPressure, vL).x;
  float R = texture(uPressure, vR).x;
  float T = texture(uPressure, vT).x;
  float B = texture(uPressure, vB).x;
  vec2 velocity = texture(uVelocity, vUv).xy;
  velocity -= vec2(R - L, T - B);
  fragColor = vec4(velocity, 0.0, 1.0);
}`,

  display: `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 fragColor;
uniform sampler2D uTexture;
uniform float uBoost;
void main () {
  vec3 c = texture(uTexture, vUv).rgb * uBoost;
  float d = distance(vUv, vec2(0.5));
  c *= smoothstep(0.95, 0.45, d) * 0.35 + 0.65;
  fragColor = vec4(c, 1.0);
}`,
};

interface FBO {
  texture: WebGLTexture;
  fbo: WebGLFramebuffer;
  width: number;
  height: number;
  texelSizeX: number;
  texelSizeY: number;
  attach: (id: number) => number;
}

interface DoubleFBO {
  width: number;
  height: number;
  texelSizeX: number;
  texelSizeY: number;
  read: FBO;
  write: FBO;
  swap: () => void;
}

interface ProgramInfo {
  p: WebGLProgram;
  u: Record<string, WebGLUniformLocation>;
  bind: () => void;
}

export class FluidSim {
  private gl: WebGL2RenderingContext | null = null;
  private canvas: HTMLCanvasElement | null = null;
  private config: FluidConfig = { ...DEFAULT_CFG };

  private programs: Record<string, ProgramInfo> = {};
  private compiledShaders: WebGLShader[] = [];
  private quadBuf: WebGLBuffer | null = null;
  private quadIdxBuf: WebGLBuffer | null = null;

  private velocity: DoubleFBO | null = null;
  private dye: DoubleFBO | null = null;
  private divergenceFBO: FBO | null = null;
  private curlFBO: FBO | null = null;
  private pressureFBO: DoubleFBO | null = null;

  public isSupported = false;

  public init(canvas: HTMLCanvasElement, customConfig?: Partial<FluidConfig>): boolean {
    this.canvas = canvas;
    if (customConfig) {
      this.config = { ...DEFAULT_CFG, ...customConfig };
    }

    const gl = canvas.getContext('webgl2', {
      alpha: false,
      depth: false,
      stencil: false,
      antialias: false,
      powerPreference: 'high-performance',
    });

    if (!gl) return false;

    const extFloat = gl.getExtension('EXT_color_buffer_float');
    if (!extFloat) return false;

    gl.getExtension('OES_texture_float_linear');

    this.gl = gl;
    this.isSupported = true;

    this.initGLResources();
    this.resize();
    return true;
  }

  private compileShader(type: number, src: string): WebGLShader {
    const gl = this.gl!;
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      console.error(gl.getShaderInfoLog(s));
    }
    this.compiledShaders.push(s);
    return s;
  }

  private makeProgram(vs: WebGLShader, fsSrc: string): ProgramInfo {
    const gl = this.gl!;
    const p = gl.createProgram()!;
    gl.attachShader(p, vs);
    const fs = this.compileShader(gl.FRAGMENT_SHADER, fsSrc);
    gl.attachShader(p, fs);
    gl.bindAttribLocation(p, 0, 'aPosition');
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
      console.error(gl.getProgramInfoLog(p));
    }

    const u: Record<string, WebGLUniformLocation> = {};
    const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
    for (let i = 0; i < n; i++) {
      const info = gl.getActiveUniform(p, i)!;
      u[info.name] = gl.getUniformLocation(p, info.name)!;
    }
    return {
      p,
      u,
      bind: () => gl.useProgram(p),
    };
  }

  private initGLResources() {
    const gl = this.gl!;
    const vs = this.compileShader(gl.VERTEX_SHADER, VERT);

    for (const [key, src] of Object.entries(FRAG)) {
      this.programs[key] = this.makeProgram(vs, src);
    }

    this.quadBuf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quadBuf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, -1, 1, 1, 1, 1, -1]), gl.STATIC_DRAW);

    this.quadIdxBuf = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.quadIdxBuf);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint16Array([0, 1, 2, 0, 2, 3]), gl.STATIC_DRAW);

    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.enableVertexAttribArray(0);
  }

  private blit(target: FBO | null) {
    const gl = this.gl!;
    if (target == null) {
      gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    } else {
      gl.viewport(0, 0, target.width, target.height);
      gl.bindFramebuffer(gl.FRAMEBUFFER, target.fbo);
    }
    gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_SHORT, 0);
  }

  private createFBO(
    w: number,
    h: number,
    internalFormat: number,
    format: number,
    type: number,
    filter: number
  ): FBO {
    const gl = this.gl!;
    gl.activeTexture(gl.TEXTURE0);
    const texture = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, internalFormat, w, h, 0, format, type, null);

    const fbo = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.viewport(0, 0, w, h);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);

    return {
      texture,
      fbo,
      width: w,
      height: h,
      texelSizeX: 1 / w,
      texelSizeY: 1 / h,
      attach(id: number) {
        gl.activeTexture(gl.TEXTURE0 + id);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        return id;
      },
    };
  }

  private createDoubleFBO(
    w: number,
    h: number,
    intF: number,
    f: number,
    t: number,
    filter: number
  ): DoubleFBO {
    let fbo1 = this.createFBO(w, h, intF, f, t, filter);
    let fbo2 = this.createFBO(w, h, intF, f, t, filter);
    return {
      width: w,
      height: h,
      texelSizeX: 1 / w,
      texelSizeY: 1 / h,
      get read() {
        return fbo1;
      },
      get write() {
        return fbo2;
      },
      swap() {
        const tmp = fbo1;
        fbo1 = fbo2;
        fbo2 = tmp;
      },
    };
  }

  private disposeFBO(fbo: FBO | null) {
    if (!fbo || !this.gl) return;
    this.gl.deleteTexture(fbo.texture);
    this.gl.deleteFramebuffer(fbo.fbo);
  }

  private disposeDoubleFBO(dfbo: DoubleFBO | null) {
    if (!dfbo) return;
    this.disposeFBO(dfbo.read);
    this.disposeFBO(dfbo.write);
  }

  public resize() {
    if (!this.gl || !this.canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, this.config.maxDpr);
    const w = Math.floor(this.canvas.clientWidth * dpr);
    const h = Math.floor(this.canvas.clientHeight * dpr);

    if (this.canvas.width === w && this.canvas.height === h && this.velocity) {
      return;
    }

    this.canvas.width = w;
    this.canvas.height = h;

    this.initFramebuffers();
  }

  private getResolution(resolution: number) {
    const gl = this.gl!;
    let ar = gl.drawingBufferWidth / gl.drawingBufferHeight;
    if (ar < 1) ar = 1 / ar;
    const min = Math.round(resolution);
    const max = Math.round(resolution * ar);
    return gl.drawingBufferWidth > gl.drawingBufferHeight
      ? { width: max, height: min }
      : { width: min, height: max };
  }

  private initFramebuffers() {
    const gl = this.gl!;
    // Clean up old WebGL framebuffers and textures to prevent memory leaks!
    this.disposeDoubleFBO(this.velocity);
    this.disposeDoubleFBO(this.dye);
    this.disposeFBO(this.divergenceFBO);
    this.disposeFBO(this.curlFBO);
    this.disposeDoubleFBO(this.pressureFBO);

    const sim = this.getResolution(this.config.simRes);
    const dyeR = this.getResolution(this.config.dyeRes);
    const HF = gl.HALF_FLOAT;

    this.velocity = this.createDoubleFBO(sim.width, sim.height, gl.RG16F, gl.RG, HF, gl.LINEAR);
    this.dye = this.createDoubleFBO(dyeR.width, dyeR.height, gl.RGBA16F, gl.RGBA, HF, gl.LINEAR);
    this.divergenceFBO = this.createFBO(sim.width, sim.height, gl.R16F, gl.RED, HF, gl.NEAREST);
    this.curlFBO = this.createFBO(sim.width, sim.height, gl.R16F, gl.RED, HF, gl.NEAREST);
    this.pressureFBO = this.createDoubleFBO(sim.width, sim.height, gl.R16F, gl.RED, HF, gl.NEAREST);
  }

  public stepSim(dt: number) {
    if (!this.gl || !this.velocity || !this.dye || !this.divergenceFBO || !this.curlFBO || !this.pressureFBO) return;
    const gl = this.gl;
    gl.disable(gl.BLEND);
    const tw = this.velocity.texelSizeX;
    const th = this.velocity.texelSizeY;

    // 1. Curl
    const pCurl = this.programs.curl;
    pCurl.bind();
    gl.uniform2f(pCurl.u.texelSize, tw, th);
    gl.uniform1i(pCurl.u.uVelocity, this.velocity.read.attach(0));
    this.blit(this.curlFBO);

    // 2. Vorticity
    const pVort = this.programs.vorticity;
    pVort.bind();
    gl.uniform2f(pVort.u.texelSize, tw, th);
    gl.uniform1i(pVort.u.uVelocity, this.velocity.read.attach(0));
    gl.uniform1i(pVort.u.uCurl, this.curlFBO.attach(1));
    gl.uniform1f(pVort.u.curl, this.config.curl);
    gl.uniform1f(pVort.u.dt, dt);
    this.blit(this.velocity.write);
    this.velocity.swap();

    // 3. Divergence
    const pDiv = this.programs.divergence;
    pDiv.bind();
    gl.uniform2f(pDiv.u.texelSize, tw, th);
    gl.uniform1i(pDiv.u.uVelocity, this.velocity.read.attach(0));
    this.blit(this.divergenceFBO);

    // 4. Clear pressure
    const pClear = this.programs.clear;
    pClear.bind();
    gl.uniform1i(pClear.u.uTexture, this.pressureFBO.read.attach(0));
    gl.uniform1f(pClear.u.value, 0.8);
    this.blit(this.pressureFBO.write);
    this.pressureFBO.swap();

    // 5. Pressure Poisson solve
    const pPress = this.programs.pressure;
    pPress.bind();
    gl.uniform2f(pPress.u.texelSize, tw, th);
    gl.uniform1i(pPress.u.uDivergence, this.divergenceFBO.attach(0));
    for (let i = 0; i < this.config.pressureIters; i++) {
      gl.uniform1i(pPress.u.uPressure, this.pressureFBO.read.attach(1));
      this.blit(this.pressureFBO.write);
      this.pressureFBO.swap();
    }

    // 6. Gradient subtract
    const pGrad = this.programs.gradient;
    pGrad.bind();
    gl.uniform2f(pGrad.u.texelSize, tw, th);
    gl.uniform1i(pGrad.u.uPressure, this.pressureFBO.read.attach(0));
    gl.uniform1i(pGrad.u.uVelocity, this.velocity.read.attach(1));
    this.blit(this.velocity.write);
    this.velocity.swap();

    // 7. Advection velocity
    const pAdv = this.programs.advection;
    pAdv.bind();
    gl.uniform2f(pAdv.u.texelSize, tw, th);
    gl.uniform1i(pAdv.u.uVelocity, this.velocity.read.attach(0));
    gl.uniform1i(pAdv.u.uSource, this.velocity.read.attach(0));
    gl.uniform1f(pAdv.u.dt, dt);
    gl.uniform1f(pAdv.u.dissipation, this.config.velDiss);
    this.blit(this.velocity.write);
    this.velocity.swap();

    // 8. Advection dye
    gl.uniform1i(pAdv.u.uVelocity, this.velocity.read.attach(0));
    gl.uniform1i(pAdv.u.uSource, this.dye.read.attach(1));
    gl.uniform1f(pAdv.u.dissipation, this.config.dyeDiss);
    this.blit(this.dye.write);
    this.dye.swap();
  }

  public splat(x: number, y: number, dx: number, dy: number, color: [number, number, number], radius: number) {
    if (!this.gl || !this.velocity || !this.dye || !this.canvas) return;
    const gl = this.gl;
    const pSplat = this.programs.splat;
    pSplat.bind();
    gl.uniform1i(pSplat.u.uTarget, this.velocity.read.attach(0));
    gl.uniform1f(pSplat.u.aspectRatio, this.canvas.width / this.canvas.height);
    gl.uniform2f(pSplat.u.point, x, y);
    gl.uniform3f(pSplat.u.color, dx, dy, 0.0);
    gl.uniform1f(pSplat.u.radius, this.correctRadius(radius));
    this.blit(this.velocity.write);
    this.velocity.swap();

    gl.uniform1i(pSplat.u.uTarget, this.dye.read.attach(0));
    gl.uniform3f(pSplat.u.color, color[0], color[1], color[2]);
    this.blit(this.dye.write);
    this.dye.swap();
  }

  private correctRadius(r: number): number {
    if (!this.canvas) return r;
    const ar = this.canvas.width / this.canvas.height;
    return ar > 1 ? r * ar : r;
  }

  public render(kickPulse: number) {
    if (!this.gl || !this.dye) return;
    const gl = this.gl;
    const pDisp = this.programs.display;
    pDisp.bind();
    gl.uniform1i(pDisp.u.uTexture, this.dye.read.attach(0));
    gl.uniform1f(pDisp.u.uBoost, 1.0 + kickPulse);
    this.blit(null);
  }

  public destroy() {
    if (!this.gl) return;
    const gl = this.gl;

    this.disposeDoubleFBO(this.velocity);
    this.disposeDoubleFBO(this.dye);
    this.disposeFBO(this.divergenceFBO);
    this.disposeFBO(this.curlFBO);
    this.disposeDoubleFBO(this.pressureFBO);

    this.velocity = null;
    this.dye = null;
    this.divergenceFBO = null;
    this.curlFBO = null;
    this.pressureFBO = null;

    if (this.quadBuf) gl.deleteBuffer(this.quadBuf);
    if (this.quadIdxBuf) gl.deleteBuffer(this.quadIdxBuf);

    for (const prog of Object.values(this.programs)) {
      gl.deleteProgram(prog.p);
    }
    for (const s of this.compiledShaders) {
      gl.deleteShader(s);
    }

    this.programs = {};
    this.compiledShaders = [];
    this.gl = null;
    this.canvas = null;
  }
}
