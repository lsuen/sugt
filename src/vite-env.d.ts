/// <reference types="vite/client" />

declare module '*.css' {
  const css: string;
  export default css;
}

declare module '*.css?url' {
  const href: string;
  export default href;
}

declare module '*.jpg' {
  const src: string;
  export default src;
}

declare module '*.png' {
  const src: string;
  export default src;
}

declare module '*.webp' {
  const src: string;
  export default src;
}
