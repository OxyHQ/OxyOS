import { useState } from 'react'
import Lightbox from './Lightbox'

const images = [
  '/images/2023-06-11--1686519304_1056x800_scrot.png',
  '/images/2023-06-11--1686519601_1042x790_scrot.png',
  '/images/2023-06-11--1686519508_1050x807_scrot.png',
  '/images/2023-06-11--1686519715_1039x785_scrot.png',
]

export default function Screenshots() {
  const [lightboxSrc, setLightboxSrc] = useState(null)

  return (
    <div className="bg-zinc-900 py-24 sm:py-32" id="screenshots">
      <div className="mx-auto max-w-7xl px-6 lg:px-8">
        <div className="mx-auto grid max-w-2xl grid-cols-1 gap-x-8 gap-y-16 sm:gap-y-20 lg:mx-0 lg:max-w-none lg:grid-cols-3">
          <div>
            <h2 className="font-semibold leading-7 text-white text-2xl">
              Screenshots
            </h2>
          </div>
          <dl className="col-span-2 grid grid-cols-1 gap-x-2 gap-y-4 text-base leading-7 text-zinc-200 sm:grid-cols-2 lg:gap-y-6">
            {images.map((src) => (
              <div key={src} className="relative pl-9">
                <button onClick={() => setLightboxSrc(src)}>
                  <img src={src} alt="Screenshot" />
                </button>
              </div>
            ))}
          </dl>
        </div>
      </div>

      {lightboxSrc && (
        <Lightbox src={lightboxSrc} onClose={() => setLightboxSrc(null)} />
      )}
    </div>
  )
}
