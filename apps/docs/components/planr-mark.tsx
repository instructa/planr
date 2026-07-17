import type { SVGProps } from 'react';

export function PlanrMark(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      <rect width="32" height="32" rx="9" fill="currentColor" className="text-planr-ink" />
      <path
        d="M9 22V10h7.2c4.4 0 6.8 2.1 6.8 5.7 0 3.8-2.7 5.9-7.1 5.9h-2.4V22H9Zm4.5-4h2.3c1.8 0 2.8-.7 2.8-2.2 0-1.4-1-2.1-2.8-2.1h-2.3V18Z"
        fill="currentColor"
        className="text-planr-mint"
      />
    </svg>
  );
}
