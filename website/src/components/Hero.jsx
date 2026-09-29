import { Link } from 'react-router-dom'
import { useReleases, getDownloadUrl } from '../hooks/useReleases'

export default function Hero() {
  const { latest } = useReleases()

  return (
    <div className="mx-auto max-w-7xl px-6 pb-24 pt-10 sm:pb-32 lg:flex lg:px-8 lg:py-40">
      <div className="mx-auto max-w-2xl flex-shrink-0 lg:mx-0 lg:max-w-xl lg:pt-8">
        <h1 className="text-4xl font-bold tracking-tight text-white sm:text-6xl mt-48">
          OxyOS
        </h1>
        <p className="mt-6 text-lg leading-8 text-zinc-300">
          A lightweight desktop Linux project. Still small, still fast.
        </p>
        <div className="mt-10 flex items-center gap-x-6">
          <a
            href={getDownloadUrl(latest, 'amd64')}
            className="rounded-md bg-stone-500 px-3.5 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-stone-400 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-stone-400"
          >
            Download ISO
          </a>
          <Link
            to="/download"
            className="text-sm font-semibold leading-6 text-white"
          >
            All Downloads <span aria-hidden="true">&rarr;</span>
          </Link>
        </div>
      </div>

      <div className="mx-auto mt-16 flex max-w-2xl sm:mt-24 lg:ml-10 lg:mr-0 lg:mt-0 lg:max-w-none lg:flex-none xl:ml-32">
        <div className="max-w-3xl flex-none sm:max-w-5xl">
          <img
            src="/images/2023-06-11--1686519304_1056x800_scrot.png"
            alt="App screenshot"
            width="1024"
            height="768"
            className="w-[76rem] bg-white/5 shadow-2xl ring-1 ring-white/10"
          />
        </div>
      </div>
    </div>
  )
}
