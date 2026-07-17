import { ArrowUpRight } from 'lucide-react';
import Link from 'next/link';
import type { ReactNode } from 'react';

type PathCardProps = {
  href: string;
  eyebrow: string;
  title: string;
  description: string;
  icon: ReactNode;
};

export function PathCard({ href, eyebrow, title, description, icon }: PathCardProps) {
  return (
    <Link className="path-card group" href={href}>
      <span className="path-card__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="path-card__eyebrow">{eyebrow}</span>
      <span className="path-card__title">
        {title}
        <ArrowUpRight aria-hidden="true" />
      </span>
      <span className="path-card__description">{description}</span>
    </Link>
  );
}
