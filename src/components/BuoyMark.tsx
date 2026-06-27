import { cn } from '@/lib/utils';

/**
 * The Quay brand mark: a miniature of the app/dock icon — a dark rounded square
 * with the white status-buoy glyph and a green beacon. Sizes via `className`
 * (defaults to a 24px square to fit the popover brand bar).
 */
export function BuoyMark({ className }: { className?: string }): React.JSX.Element {
	return (
		<svg
			viewBox="0 0 24 24"
			className={cn('size-6', className)}
			role="img"
			aria-label="Quay"
		>
			<defs>
				<linearGradient id="quay-mark-bg" x1="0" y1="0" x2="0" y2="1">
					<stop offset="0" stopColor="#2A3340" />
					<stop offset="1" stopColor="#0E131B" />
				</linearGradient>
			</defs>
			<rect x="0" y="0" width="24" height="24" rx="6" fill="url(#quay-mark-bg)" />
			{/* 22-unit buoy glyph centered into the 24px tile. */}
			<g
				transform="translate(-0.65 -0.08) scale(1.15)"
				fill="none"
				stroke="#ffffff"
				strokeWidth="1.4"
				strokeLinecap="round"
				strokeLinejoin="round"
			>
				<path d="M9 7h4" />
				<path d="M8 11h6" />
				<path d="M8.5 7l-2 9h9l-2-9" />
				<path d="M4.5 18c1.4-.9 2.8-.9 4.2 0s2.8.9 4.2 0 2.8-.9 4.6 0" />
				{/* Green status beacon. */}
				<circle cx="11" cy="4.5" r="1.4" fill="#34D058" stroke="none" />
			</g>
		</svg>
	);
}
