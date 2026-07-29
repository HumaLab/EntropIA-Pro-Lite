import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import MapViewer from '../MapViewer.svelte'
import type { MapMarker } from '../MapViewer.types'

const leafletMock = vi.hoisted(() => {
  type LatLng = { lat: number; lng: number }
  type Handler = () => void

  const markers: Array<{
    options: { draggable?: boolean }
    dragging: { enable: ReturnType<typeof vi.fn>; disable: ReturnType<typeof vi.fn> }
    setLatLng: ReturnType<typeof vi.fn>
    getLatLng: () => LatLng
    moveTo: (latitude: number, longitude: number) => void
    trigger: (event: string) => void
  }> = []

  const mapInstance = {
    setView: vi.fn(),
    invalidateSize: vi.fn(),
    fitBounds: vi.fn(),
    remove: vi.fn(),
  }
  mapInstance.setView.mockReturnValue(mapInstance)

  const layer = {
    addTo: vi.fn(),
    clearLayers: vi.fn(),
  }
  layer.addTo.mockReturnValue(layer)

  const leaflet = {
    Icon: {
      Default: {
        prototype: {},
        mergeOptions: vi.fn(),
      },
    },
    map: vi.fn(() => mapInstance),
    tileLayer: vi.fn(() => ({ addTo: vi.fn() })),
    layerGroup: vi.fn(() => layer),
    latLng: vi.fn((latitude: number, longitude: number) => ({ lat: latitude, lng: longitude })),
    latLngBounds: vi.fn((bounds: LatLng[]) => bounds),
    marker: vi.fn((initial: LatLng, options: { draggable?: boolean }) => {
      let current = { ...initial }
      const handlers = new Map<string, Handler>()
      const marker = {
        options,
        dragging: { enable: vi.fn(), disable: vi.fn() },
        addTo: vi.fn(),
        bindPopup: vi.fn(),
        on: vi.fn((event: string, handler: Handler) => {
          handlers.set(event, handler)
          return marker
        }),
        setLatLng: vi.fn((next: [number, number]) => {
          current = { lat: next[0], lng: next[1] }
          return marker
        }),
        getLatLng: () => current,
        moveTo: (latitude: number, longitude: number) => {
          current = { lat: latitude, lng: longitude }
        },
        trigger: (event: string) => handlers.get(event)?.(),
      }
      marker.addTo.mockReturnValue(marker)
      marker.bindPopup.mockReturnValue(marker)
      markers.push(marker)
      return marker
    }),
  }

  return { layer, leaflet, mapInstance, markers }
})

vi.mock('leaflet', () => ({ default: leafletMock.leaflet }))

class MockResizeObserver {
  observe = vi.fn()
  disconnect = vi.fn()
}

vi.stubGlobal('ResizeObserver', MockResizeObserver)
vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
  callback(0)
  return 1
})

const buenosAires: MapMarker = {
  entityId: 'place-1',
  label: 'Buenos Aires',
  latitude: -34.6037,
  longitude: -58.3816,
}

describe('MapViewer location editing', () => {
  beforeEach(() => {
    leafletMock.markers.length = 0
    vi.clearAllMocks()
  })

  it('keeps markers locked until Edit location is activated', async () => {
    render(MapViewer, { props: { markers: [buenosAires], onlocationchange: vi.fn() } })

    await waitFor(() => expect(leafletMock.markers).toHaveLength(1))
    const marker = leafletMock.markers[0]!
    expect(marker.options.draggable).toBe(false)
    expect(marker.dragging.enable).not.toHaveBeenCalled()

    await fireEvent.click(screen.getByRole('button', { name: 'Edit location' }))
    expect(marker.dragging.enable).toHaveBeenCalledOnce()
  })

  it('captures signed marker coordinates on dragend and saves them', async () => {
    const onlocationchange = vi.fn().mockResolvedValue(undefined)
    render(MapViewer, { props: { markers: [buenosAires], onlocationchange } })

    await waitFor(() => expect(leafletMock.markers).toHaveLength(1))
    await fireEvent.click(screen.getByRole('button', { name: 'Edit location' }))

    const marker = leafletMock.markers[0]!
    marker.moveTo(-34.615, -58.433)
    marker.trigger('dragend')

    await fireEvent.click(await screen.findByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(onlocationchange).toHaveBeenCalledWith('place-1', -34.615, -58.433)
    )
  })

  it('restores the prior marker position when editing is cancelled', async () => {
    const onlocationchange = vi.fn()
    render(MapViewer, { props: { markers: [buenosAires], onlocationchange } })

    await waitFor(() => expect(leafletMock.markers).toHaveLength(1))
    await fireEvent.click(screen.getByRole('button', { name: 'Edit location' }))

    const marker = leafletMock.markers[0]!
    marker.moveTo(-33, -57)
    marker.trigger('dragend')
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))

    expect(marker.setLatLng).toHaveBeenCalledWith([-34.6037, -58.3816])
    expect(marker.dragging.disable).toHaveBeenCalledOnce()
    expect(onlocationchange).not.toHaveBeenCalled()
  })

  it('offers restoring automatic coordinates only for a manual location', async () => {
    const onresetlocation = vi.fn().mockResolvedValue(undefined)
    render(MapViewer, {
      props: {
        markers: [{ ...buenosAires, hasManualLocation: true }],
        onlocationchange: vi.fn(),
        onresetlocation,
      },
    })

    await fireEvent.click(
      await screen.findByRole('button', { name: 'Reset automatic location' })
    )
    await waitFor(() => expect(onresetlocation).toHaveBeenCalledWith('place-1'))
  })
})
