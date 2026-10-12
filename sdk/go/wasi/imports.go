//go:build wasip1

// Package wasi calls wasi:io and wasi:sockets 0.2 through the canonical ABI,
// so a wasip1 Go module, made a component by cmd/componentize, can open TCP
// connections. The interfaces are declared in ../wit/go.wit.
package wasi

import "unsafe"

//go:wasmimport wasi:sockets/instance-network@0.2.12 instance-network
func instanceNetwork() uint32

//go:wasmimport wasi:sockets/tcp-create-socket@0.2.12 create-tcp-socket
func createTCPSocket(family uint32, ret unsafe.Pointer)

//go:wasmimport wasi:sockets/tcp@0.2.12 [method]tcp-socket.start-connect
func startConnect(sock, network, disc, a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10 uint32, ret unsafe.Pointer)

//go:wasmimport wasi:sockets/tcp@0.2.12 [method]tcp-socket.finish-connect
func finishConnect(sock uint32, ret unsafe.Pointer)

//go:wasmimport wasi:sockets/tcp@0.2.12 [method]tcp-socket.subscribe
func tcpSubscribe(sock uint32) uint32

//go:wasmimport wasi:sockets/tcp@0.2.12 [method]tcp-socket.shutdown
func tcpShutdown(sock, how uint32, ret unsafe.Pointer)

//go:wasmimport wasi:sockets/tcp@0.2.12 [resource-drop]tcp-socket
func dropTCPSocket(sock uint32)

//go:wasmimport wasi:sockets/ip-name-lookup@0.2.12 resolve-addresses
func resolveAddresses(network uint32, name unsafe.Pointer, nameLen uint32, ret unsafe.Pointer)

//go:wasmimport wasi:sockets/ip-name-lookup@0.2.12 [method]resolve-address-stream.resolve-next-address
func resolveNextAddress(stream uint32, ret unsafe.Pointer)

//go:wasmimport wasi:sockets/ip-name-lookup@0.2.12 [method]resolve-address-stream.subscribe
func resolveSubscribe(stream uint32) uint32

//go:wasmimport wasi:sockets/ip-name-lookup@0.2.12 [resource-drop]resolve-address-stream
func dropResolveStream(stream uint32)

//go:wasmimport wasi:io/streams@0.2.12 [method]input-stream.read
func streamRead(stream uint32, n uint64, ret unsafe.Pointer)

//go:wasmimport wasi:io/streams@0.2.12 [method]input-stream.subscribe
func inputSubscribe(stream uint32) uint32

//go:wasmimport wasi:io/streams@0.2.12 [resource-drop]input-stream
func dropInputStream(stream uint32)

//go:wasmimport wasi:io/streams@0.2.12 [method]output-stream.check-write
func checkWrite(stream uint32, ret unsafe.Pointer)

//go:wasmimport wasi:io/streams@0.2.12 [method]output-stream.write
func streamWrite(stream uint32, buf unsafe.Pointer, n uint32, ret unsafe.Pointer)

//go:wasmimport wasi:io/streams@0.2.12 [method]output-stream.flush
func streamFlush(stream uint32, ret unsafe.Pointer)

//go:wasmimport wasi:io/streams@0.2.12 [method]output-stream.subscribe
func outputSubscribe(stream uint32) uint32

//go:wasmimport wasi:io/streams@0.2.12 [resource-drop]output-stream
func dropOutputStream(stream uint32)

//go:wasmimport wasi:io/error@0.2.12 [resource-drop]error
func dropError(e uint32)

//go:wasmimport wasi:io/poll@0.2.12 poll
func poll(list unsafe.Pointer, n uint32, ret unsafe.Pointer)

//go:wasmimport wasi:io/poll@0.2.12 [resource-drop]pollable
func dropPollable(p uint32)

//go:wasmimport wasi:clocks/monotonic-clock@0.2.12 subscribe-duration
func subscribeDuration(ns uint64) uint32
