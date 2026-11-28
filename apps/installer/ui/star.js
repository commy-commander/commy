"use strict";

const CommyStar = (() => {
  const COUNT = 15000;
  const HALO_SHARE = 0.15;
  const FOV = 34 * Math.PI / 180;
  const CAMERA = [0, 0.1, 6.4];
  const LIFT = 0.42;

  const STAR = Array.from({ length: 10 }, (_, i) => {
    const angle = Math.PI / 2 + i * Math.PI / 5;
    const radius = i % 2 === 0 ? 1 : 0.5;
    return [Math.cos(angle) * radius, Math.sin(angle) * radius];
  });

  function insideStar(x, y) {
    let inside = false;
    for (let i = 0, j = STAR.length - 1; i < STAR.length; j = i++) {
      const [ax, ay] = STAR[i];
      const [bx, by] = STAR[j];
      if ((ay > y) !== (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax) inside = !inside;
    }
    return inside;
  }

  function pointInStar() {
    let x;
    let y;
    do {
      x = Math.random() * 2 - 1;
      y = Math.random() * 2 - 1;
    } while (!insideStar(x, y));
    return [x, y];
  }

  function gauss() {
    return (Math.random() + Math.random() + Math.random() - 1.5) / 1.5;
  }

  function haloSpot() {
    const [x, y] = pointInStar();
    const spread = 1.08 + Math.random() * 0.5;
    return [x * spread, y * spread, 0];
  }

  function shellSpot() {
    let [x, y] = pointInStar();
    const soft = 1 + gauss() * 0.05;
    x *= soft;
    y *= soft;
    const angle = Math.atan2(x, y);
    const reach = 0.5 + 0.5 * Math.pow(0.5 + 0.5 * Math.cos(5 * angle), 1.6);
    const share = Math.min(1, Math.hypot(x, y) / Math.max(reach, 0.001));
    const push = share > 0 ? Math.pow(share, 0.4) / share : 1;
    const puff = Math.sqrt(Math.max(0, 1 - Math.min(1, share * push) ** 2));
    const side = Math.random() < 0.5 ? -1 : 1;
    return [x * push, y * push, side * puff * 0.55 * (0.85 + Math.random() * 0.15)];
  }

  function particles() {
    const spots = new Float32Array(COUNT * 4);
    for (let i = 0; i < COUNT; i++) {
      const spot = Math.random() < HALO_SHARE ? haloSpot() : shellSpot();
      spots.set([...spot, Math.random()], i * 4);
    }
    return spots;
  }

  const NOISE = `
    vec3 hash3(vec3 p) {
      p = vec3(dot(p, vec3(127.1, 311.7, 74.7)), dot(p, vec3(269.5, 183.3, 246.1)), dot(p, vec3(113.5, 271.9, 124.6)));
      return fract(sin(p) * 43758.5453) * 2.0 - 1.0;
    }
    float noise(vec3 p) {
      vec3 i = floor(p);
      vec3 f = fract(p);
      vec3 u = f * f * (3.0 - 2.0 * f);
      return mix(mix(mix(dot(hash3(i), f), dot(hash3(i + vec3(1, 0, 0)), f - vec3(1, 0, 0)), u.x),
                     mix(dot(hash3(i + vec3(0, 1, 0)), f - vec3(0, 1, 0)), dot(hash3(i + vec3(1, 1, 0)), f - vec3(1, 1, 0)), u.x), u.y),
                 mix(mix(dot(hash3(i + vec3(0, 0, 1)), f - vec3(0, 0, 1)), dot(hash3(i + vec3(1, 0, 1)), f - vec3(1, 0, 1)), u.x),
                     mix(dot(hash3(i + vec3(0, 1, 1)), f - vec3(0, 1, 1)), dot(hash3(i + vec3(1, 1, 1)), f - vec3(1, 1, 1)), u.x), u.y), u.z);
    }`;

  const VERTEX = `
    precision highp float;
    attribute vec4 aSpot;
    uniform mat4 uProjection;
    uniform mat4 uView;
    uniform float uTime;
    uniform float uEnergy;
    uniform float uHue;
    uniform float uScale;
    varying vec3 vColor;
    varying float vAlpha;
    ${NOISE}
    void main() {
      vec3 spot = aSpot.xyz;
      float seed = aSpot.w;
      vec3 dir = normalize(spot + vec3(0.0, 0.0, 0.0001));
      float wave = noise(dir * 1.6 + vec3(0.0, uTime * 0.35, uTime * 0.2));
      float ripple = noise(dir * 4.0 - vec3(uTime * 0.5));
      vec3 place = spot * vec3(1.12, 1.12, 1.0) + dir * (wave * 0.22 * uEnergy + ripple * 0.04);
      vec4 seen = uView * vec4(place, 1.0);
      gl_Position = uProjection * seen;
      float sparkle = seed > 0.985 ? 2.6 : 1.0;
      gl_PointSize = min(30.0 * uScale * (0.35 + seed * 0.55) * sparkle * 1.35 / -seen.z, 5.0 * uScale);
      float shade = clamp(noise(place * 1.3 + vec3(uTime * 0.22, -uTime * 0.18, uTime * 0.15)) * 0.9 + 0.5 + (seed - 0.5) * 0.3, 0.0, 1.0);
      vec3 early = mix(vec3(0.16, 0.27, 0.95), vec3(0.43, 0.525, 1.0), shade);
      vec3 late = mix(vec3(0.36, 0.46, 1.0), vec3(0.82, 0.86, 1.0), shade);
      vColor = mix(mix(early, late, uHue), vec3(1.0, 0.72, 0.32), smoothstep(0.72, 0.98, shade) * uHue * 0.6);
      vAlpha = 0.95 * (0.55 + 0.45 * sin(uTime * (1.2 + seed * 2.5) + seed * 40.0));
    }`;

  const FRAGMENT = `
    precision mediump float;
    varying vec3 vColor;
    varying float vAlpha;
    void main() {
      float glow = smoothstep(0.5, 0.0, length(gl_PointCoord - 0.5));
      float alpha = pow(glow, 2.2) * vAlpha;
      if (alpha < 0.003) discard;
      gl_FragColor = vec4(vColor * alpha, alpha);
    }`;

  function perspective(aspect) {
    const f = 1 / Math.tan(FOV / 2);
    const near = 0.1;
    const far = 50;
    return new Float32Array([f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) / (near - far), -1, 0, 0, (2 * far * near) / (near - far), 0]);
  }

  function multiply(a, b) {
    const out = new Float32Array(16);
    for (let col = 0; col < 4; col++) {
      for (let row = 0; row < 4; row++) {
        let sum = 0;
        for (let k = 0; k < 4; k++) sum += a[k * 4 + row] * b[col * 4 + k];
        out[col * 4 + row] = sum;
      }
    }
    return out;
  }

  function view(yaw, pitch) {
    const turnY = new Float32Array([Math.cos(yaw), 0, -Math.sin(yaw), 0, 0, 1, 0, 0, Math.sin(yaw), 0, Math.cos(yaw), 0, 0, 0, 0, 1]);
    const turnX = new Float32Array([1, 0, 0, 0, 0, Math.cos(pitch), Math.sin(pitch), 0, 0, -Math.sin(pitch), Math.cos(pitch), 0, 0, 0, 0, 1]);
    const lift = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, LIFT, 0, 1]);
    const camera = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -CAMERA[0], -CAMERA[1], -CAMERA[2], 1]);
    return multiply(camera, multiply(lift, multiply(turnX, turnY)));
  }

  function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) || "shader failed");
    return shader;
  }

  function program(gl) {
    const linked = gl.createProgram();
    gl.attachShader(linked, compile(gl, gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(linked, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(linked);
    if (!gl.getProgramParameter(linked, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(linked) || "link failed");
    return linked;
  }

  function start(canvas, { still = false } = {}) {
    const gl = canvas.getContext("webgl", { alpha: true, premultipliedAlpha: true, antialias: true });
    if (!gl) return null;
    const linked = program(gl);
    gl.useProgram(linked);
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, particles(), gl.STATIC_DRAW);
    const spot = gl.getAttribLocation(linked, "aSpot");
    gl.enableVertexAttribArray(spot);
    gl.vertexAttribPointer(spot, 4, gl.FLOAT, false, 0, 0);
    const at = (name) => gl.getUniformLocation(linked, name);
    const uniforms = { projection: at("uProjection"), view: at("uView"), time: at("uTime"), energy: at("uEnergy"), hue: at("uHue"), scale: at("uScale") };
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE);
    gl.clearColor(0, 0, 0, 0);

    const wanted = { hue: 0, energy: 0.9 };
    const shown = { hue: 0, energy: 0.9, yaw: 0 };
    let last = 0;
    let frame = 0;

    function fit() {
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const width = Math.round(canvas.clientWidth * ratio);
      const height = Math.round(canvas.clientHeight * ratio);
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      gl.viewport(0, 0, width, height);
      gl.uniformMatrix4fv(uniforms.projection, false, perspective(width / Math.max(height, 1)));
      gl.uniform1f(uniforms.scale, ratio * (canvas.clientHeight / 400));
    }

    function draw(now) {
      const t = now / 1000;
      const step = Math.min(0.05, Math.max(0, t - last));
      last = t;
      shown.hue += (wanted.hue - shown.hue) * Math.min(1, step * 3);
      shown.energy += (wanted.energy - shown.energy) * Math.min(1, step * 2);
      const yaw = Math.sin(t * 0.35) * (0.35 - shown.hue * 0.2);
      shown.yaw += (yaw - shown.yaw) * Math.min(1, step * 2);
      gl.uniformMatrix4fv(uniforms.view, false, view(shown.yaw, Math.sin(t * 0.4) * 0.05));
      gl.uniform1f(uniforms.time, t);
      gl.uniform1f(uniforms.energy, shown.energy);
      gl.uniform1f(uniforms.hue, shown.hue);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.POINTS, 0, COUNT);
    }

    function loop(now) {
      draw(now);
      frame = requestAnimationFrame(loop);
    }

    function resume() {
      cancelAnimationFrame(frame);
      if (still) {
        shown.hue = wanted.hue;
        shown.energy = wanted.energy;
        draw(2000);
      } else if (!document.hidden) {
        frame = requestAnimationFrame(loop);
      }
    }

    fit();
    window.addEventListener("resize", () => { fit(); resume(); });
    document.addEventListener("visibilitychange", resume);
    resume();

    return {
      set(next) {
        Object.assign(wanted, next);
        if (still) resume();
      },
    };
  }

  return { start };
})();
