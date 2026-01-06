# Dashboard Frontend Guidelines

## Styling

**Use inline styles in React components instead of adding to `app/styles.css`.**

The global stylesheet (`app/styles.css`) should **only** contain Tailwind theme variables (`@theme` block and `:root` CSS custom properties). Nothing else—no element selectors, no class selectors, no pseudo-selectors.

For all other styles, use inline styles or component-scoped CSS. This makes style interactions easier to understand and prevents unintended side effects from global CSS changes.

### Examples

**Prefer:**
```tsx
// Inline styles via style prop
<div style={{ transformOrigin: "top", transition: "opacity 0.1s ease-out" }}>

// Or CSS-in-JS constants
const dropdownStyle: CSSProperties = {
  transformOrigin: "top",
  transition: "opacity 0.1s ease-out",
};
<div style={dropdownStyle}>

// Or inject scoped styles via <style> tag for CSS-only features
const STYLES = `
.my-component { view-transition-name: my-transition; }
`;
<style>{STYLES}</style>
```

**Avoid:**
```css
/* Adding to app/styles.css */
.my-component {
  transform-origin: top;
  transition: opacity 0.1s ease-out;
}

html, body { ... }
:focus-visible { ... }
```

### CSS-Only Features

Some CSS features cannot be expressed as inline styles:
- View Transitions API (`view-transition-name`)
- Keyframe animations (`@keyframes`)
- Pseudo-elements (`::before`, `::after`)
- Pseudo-selectors (`:focus-visible`, `:hover` for complex cases)
- CSS custom properties with complex values (e.g., gradients) that don't serialize correctly in React's style prop

For these, use React 19's `<style>` component with `href` and `precedence` props for scoped, deduplicated styles.

### Scoped Styles with `useId` (React 19 Pattern)

When a component needs CSS-only features, use `useId()` to scope styles to a specific element:

```tsx
import { useId } from "react";

function MyComponent({ children }) {
  const id = useId();

  const styles = `
#${id}::before {
  content: "";
  position: absolute;
  /* ... */
}
`;

  return (
    <>
      <style href={`MyComponent-${id}`} precedence="medium">{styles}</style>
      <div id={id}>{children}</div>
    </>
  );
}
```

**Key points:**
- `useId()` generates a stable, hydration-safe ID (e.g., `:r0:`)
- Use `#${id}` in CSS to scope styles to that specific element
- `href` must be **globally unique** — React deduplicates styles by `href`, so if two different style blocks share the same `href`, only the first one is kept. Include `${id}` in the `href` to ensure uniqueness per component instance.
- `precedence` controls cascade order: `"low"`, `"medium"`, `"high"`
- React hoists these `<style>` tags to `<head>` automatically

See `Noise.tsx` for a complete example, or `root.tsx` for root element styles (`html`, `body`, `:focus-visible`).
