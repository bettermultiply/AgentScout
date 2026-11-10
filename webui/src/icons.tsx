import type { ReactNode } from 'react'
import type { BranchStatus } from './types'
import type { SpeculationReason } from './fold'

interface IconProps {
  size?: number
}

function Svg({ size = 16, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  )
}

export function StatusIcon({
  status,
  size = 16,
}: IconProps & { status: BranchStatus }) {
  switch (status) {
    case 'created':
      return (
        <Svg size={size}>
          <circle cx="12" cy="12" r="7" />
        </Svg>
      )
    case 'running':
      return (
        <Svg size={size}>
          <path d="M8 5.5v13l10-6.5z" fill="currentColor" stroke="none" />
        </Svg>
      )
    case 'awaiting_validation':
      return (
        <Svg size={size}>
          <path d="M7 3h10M7 21h10M8 3c0 4 1.5 6 4 9-2.5 3-4 5-4 9M16 3c0 4-1.5 6-4 9 2.5 3 4 5 4 9" />
        </Svg>
      )
    case 'promoting':
      return (
        <Svg size={size}>
          <path d="M12 19V5M6 11l6-6 6 6" />
        </Svg>
      )
    case 'promoted':
      return (
        <Svg size={size}>
          <path d="M5 12.5l4.5 4.5L19 7" />
        </Svg>
      )
    case 'discarding':
      return (
        <Svg size={size}>
          <path d="M12 5v14M6 13l6 6 6-6" />
        </Svg>
      )
    case 'discarded':
      return (
        <Svg size={size}>
          <path d="M6 6l12 12M18 6L6 18" />
        </Svg>
      )
    case 'cleaning':
      return (
        <Svg size={size}>
          <path d="M20 11a8 8 0 1 0-2.3 6.7M20 5v6h-6" />
        </Svg>
      )
    case 'cleaned':
      return (
        <Svg size={size}>
          <circle cx="12" cy="12" r="8" />
          <path d="M8.5 12.5l2.5 2.5 4.8-5" />
        </Svg>
      )
  }
}

export function ReasonIcon({
  reason,
  size = 14,
}: IconProps & { reason: SpeculationReason }) {
  switch (reason) {
    case 'tool':
      return (
        <Svg size={size}>
          <path d="M14.7 6.3a4.4 4.4 0 0 0-5.9 5.4L4 16.5V20h3.5l4.8-4.8a4.4 4.4 0 0 0 5.4-5.9l-2.8 2.8-2.4-.6-.6-2.4z" />
        </Svg>
      )
    case 'model':
      return (
        <Svg size={size}>
          <path d="M12 3l1.7 5.3L19 10l-5.3 1.7L12 17l-1.7-5.3L5 10l5.3-1.7z" />
          <path d="M19 16l.8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8z" />
        </Svg>
      )
    case 'other':
      return (
        <Svg size={size}>
          <circle cx="12" cy="12" r="8" />
          <path d="M9.8 9.3a2.3 2.3 0 1 1 3.7 1.9c-.9.7-1.5 1.2-1.5 2.3" />
          <path d="M12 17h.01" />
        </Svg>
      )
  }
}

export function PanelIcon({ size = 18 }: IconProps) {
  return (
    <Svg size={size}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M14.5 4v16" />
    </Svg>
  )
}

export function SendIcon({ size = 16 }: IconProps) {
  return (
    <Svg size={size}>
      <path d="M4 12l16-7-5 16-3.5-6.5z" />
      <path d="M11.5 14.5L20 5" />
    </Svg>
  )
}

export function BackIcon({ size = 16 }: IconProps) {
  return (
    <Svg size={size}>
      <path d="M15 5l-7 7 7 7" />
    </Svg>
  )
}

export function CloseIcon({ size = 16 }: IconProps) {
  return (
    <Svg size={size}>
      <path d="M6 6l12 12M18 6L6 18" />
    </Svg>
  )
}

export function LatestIcon({ size = 16 }: IconProps) {
  return (
    <Svg size={size}>
      <path d="M12 4v13M6 11l6 6 6-6" />
    </Svg>
  )
}
