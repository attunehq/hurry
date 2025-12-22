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
- Pseudo-elements (`:before`, `:after`)
- Pseudo-selectors (`:focus-visible`, `:hover` for complex cases)

For these, inject scoped styles via a `<style>` tag in the component. See `TabContent.tsx` and `Noise.tsx` for examples, or `root.tsx` for app-wide styles like `:focus-visible`.
