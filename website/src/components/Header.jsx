import { useState } from 'react'
import { Link } from 'react-router-dom'

export default function Header() {
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)

  return (
    <header>
      <nav
        className="mx-auto flex max-w-7xl items-center justify-between p-6 lg:px-8"
        aria-label="Global"
      >
        <div className="flex lg:flex-1">
          <Link to="/" className="-m-1.5 p-1.5 text-white text-xl">
            Oxy<span className="text-stone-400">OS</span>
          </Link>
        </div>
        <div className="flex lg:hidden">
          <button
            type="button"
            className="-m-2.5 inline-flex items-center justify-center rounded-md p-2.5 text-zinc-400"
            onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
          >
            <span className="sr-only">Open main menu</span>
            {mobileMenuOpen ? (
              <svg
                className="h-6 w-6"
                fill="none"
                viewBox="0 0 24 24"
                strokeWidth="1.5"
                stroke="currentColor"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            ) : (
              <svg
                className="h-6 w-6"
                fill="none"
                viewBox="0 0 24 24"
                strokeWidth="1.5"
                stroke="currentColor"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5"
                />
              </svg>
            )}
          </button>
        </div>
        <div className="hidden lg:flex lg:gap-x-12">
          <a href="/#features" className="text-sm font-semibold leading-6 text-white">
            Features
          </a>
          <a href="/#screenshots" className="text-sm font-semibold leading-6 text-white">
            Screenshots
          </a>
          <a href="/#faq" className="text-sm font-semibold leading-6 text-white">
            FAQ
          </a>
          <Link to="/download" className="text-sm font-semibold leading-6 text-white">
            Download
          </Link>
          <a href="https://github.com/OxyHQ/OxyOS" className="text-sm font-semibold leading-6 text-white">
            Github
          </a>
        </div>
      </nav>

      {mobileMenuOpen && (
        <div className="lg:hidden">
          <div className="space-y-1 px-6 pb-6">
            <a
              href="/#features"
              className="block text-sm font-semibold leading-6 text-white py-2"
              onClick={() => setMobileMenuOpen(false)}
            >
              Features
            </a>
            <a
              href="/#screenshots"
              className="block text-sm font-semibold leading-6 text-white py-2"
              onClick={() => setMobileMenuOpen(false)}
            >
              Screenshots
            </a>
            <a
              href="/#faq"
              className="block text-sm font-semibold leading-6 text-white py-2"
              onClick={() => setMobileMenuOpen(false)}
            >
              FAQ
            </a>
            <Link
              to="/download"
              className="block text-sm font-semibold leading-6 text-white py-2"
              onClick={() => setMobileMenuOpen(false)}
            >
              Download
            </Link>
            <a
              href="https://github.com/OxyHQ/OxyOS"
              className="block text-sm font-semibold leading-6 text-white py-2"
            >
              Github
            </a>
          </div>
        </div>
      )}
    </header>
  )
}
