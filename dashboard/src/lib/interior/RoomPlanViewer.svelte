<script lang="ts">
  import { onMount } from "svelte";
  import * as THREE from "three";
  import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
  import { USDZLoader } from "three/examples/jsm/loaders/USDZLoader.js";

  export let assetUrl: string;

  let host: HTMLDivElement;
  let error: string | null = null;

  onMount(() => {
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0xf7f9f8);
    const camera = new THREE.PerspectiveCamera(35, 1, 0.01, 1000);
    camera.position.set(3, 2.4, 3);

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    host.appendChild(renderer.domElement);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.target.set(0, 1, 0);

    scene.add(new THREE.HemisphereLight(0xffffff, 0x65736d, 2.2));
    const key = new THREE.DirectionalLight(0xffffff, 2.5);
    key.position.set(4, 6, 5);
    scene.add(key);

    const loader = new USDZLoader();
    let frame = 0;
    let model: THREE.Object3D | null = null;

    const resize = () => {
      const width = host.clientWidth;
      const height = Math.max(host.clientHeight, 280);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height, false);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    resize();

    loader.load(
      assetUrl,
      (loaded) => {
        model = loaded;
        const bounds = new THREE.Box3().setFromObject(loaded);
        const center = bounds.getCenter(new THREE.Vector3());
        const size = bounds.getSize(new THREE.Vector3());
        const extent = Math.max(size.x, size.y, size.z, 1);
        loaded.position.sub(center);
        scene.add(loaded);
        camera.position.set(extent * 1.8, extent * 1.35, extent * 1.8);
        camera.near = extent / 100;
        camera.far = extent * 20;
        camera.updateProjectionMatrix();
        controls.target.set(0, 0, 0);
        controls.update();
      },
      undefined,
      () => {
        error = "The native USDZ could not be rendered in this browser.";
      },
    );

    const render = () => {
      controls.update();
      renderer.render(scene, camera);
      frame = requestAnimationFrame(render);
    };
    render();

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      controls.dispose();
      renderer.dispose();
      if (model) scene.remove(model);
      renderer.domElement.remove();
    };
  });
</script>

<div class="viewer" bind:this={host} aria-label="Interactive 3D RoomPlan model">
  {#if error}<p class="error">{error}</p>{/if}
</div>

<style>
  .viewer {
    position: relative;
    min-height: 280px;
    overflow: hidden;
    border: 1px solid var(--line, #d7dedb);
    border-radius: 14px;
    background: #f7f9f8;
  }
  :global(canvas) { display: block; width: 100%; height: 100%; }
  .error {
    position: absolute;
    z-index: 1;
    inset: 1rem;
    color: #8f3b3b;
  }
</style>
