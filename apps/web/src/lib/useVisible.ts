import { useEffect, useRef, useState } from "react";

/** Becomes true once the element scrolls near the viewport, and stays true. */
export function useVisible<T extends Element>(margin = "300px") {
  const ref = useRef<T>(null);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || visible) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        setVisible(true);
        observer.disconnect();
      }
    }, { rootMargin: margin });
    observer.observe(el);
    return () => observer.disconnect();
  }, [visible, margin]);
  return [ref, visible] as const;
}
