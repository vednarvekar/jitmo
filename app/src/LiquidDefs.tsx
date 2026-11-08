/** SVG filter used by the glass in styles.css: slow turbulence that bends the
 *  colour moving behind the bar, so it looks like liquid refracting light. */
export default function LiquidDefs() {
  return (
    <svg width="0" height="0" style={{ position: "absolute" }} aria-hidden="true" focusable="false">
      <defs>
        <filter id="lg-refract" x="-20%" y="-60%" width="140%" height="220%" colorInterpolationFilters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="0.006 0.014" numOctaves="2" seed="7" result="noise">
            <animate
              attributeName="baseFrequency"
              dur="26s"
              values="0.006 0.014;0.011 0.008;0.006 0.014"
              repeatCount="indefinite"
            />
          </feTurbulence>
          <feDisplacementMap in="SourceGraphic" in2="noise" scale="46" xChannelSelector="R" yChannelSelector="G" />
        </filter>
      </defs>
    </svg>
  );
}
