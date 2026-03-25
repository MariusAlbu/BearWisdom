import type React from 'react'
import styles from './Header.module.css'

interface SearchInputProps {
  value: string
  placeholder: string
  searching: boolean
  inputRef: React.RefObject<HTMLInputElement | null>
  onChange: (e: React.ChangeEvent<HTMLInputElement>) => void
  onKeyDown: (e: React.KeyboardEvent<HTMLInputElement>) => void
  onFocus: () => void
}

export function SearchInput({
  value,
  placeholder,
  searching,
  inputRef,
  onChange,
  onKeyDown,
  onFocus,
}: SearchInputProps) {
  return (
    <div className={styles.searchWrapper}>
      <span className={styles.searchIcon}>
        <svg
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
        >
          <circle cx="7" cy="7" r="5" />
          <path d="M11 11l3 3" strokeLinecap="round" />
        </svg>
      </span>
      <input
        ref={inputRef}
        className={styles.searchInput}
        type="text"
        placeholder={placeholder}
        value={value}
        onChange={onChange}
        onKeyDown={onKeyDown}
        onFocus={onFocus}
        aria-label={placeholder}
      />
      {searching && <span className={styles.searchSpinner} />}
    </div>
  )
}
