package main

import (
	"fmt"
	"log"
	"net"
	"strings"
	"time"
	"unicode/utf8"
)

const (
	SafeMode    = true
	Port        = "6969"
	MessageRate = 1.0
	BanLimit    = 10.0
	StrikeLimit = 10
)

func sensitive(message string) string {
	if SafeMode {
		return "[REDACTED]"
	} else {
		return message
	}
}

type MessageType int

const (
	ClientConnected MessageType = iota + 1
	NewMessage
	ClientDisconnected
)

type Message struct {
	Name *string
	Type MessageType
	Conn net.Conn
	Text string
}

type Client struct {
	Name        *string
	Conn        net.Conn
	LastMessage time.Time
	StrikeCount int
}

func cleanName(name string) string {
	trimmed := strings.TrimSpace(name)
	fmt.Println("trimmed: ", trimmed)
	if utf8.ValidString(trimmed) && trimmed != "" {
		return trimmed
	}
	return "anon"
}

func server(messages chan Message) {
	clients := map[string]*Client{}
	bannedMfs := map[string]time.Time{}
	for {
		msg := <-messages
		switch msg.Type {
		case ClientConnected:
			addr := msg.Conn.RemoteAddr().(*net.TCPAddr)
			bannedAt, banned := bannedMfs[addr.IP.String()]
			now := time.Now()
			name := msg.Name
			if banned {
				if now.Sub(bannedAt).Seconds() >= BanLimit {
					delete(bannedMfs, addr.IP.String())
					banned = false
				}
			}

			if !banned {
				clients[msg.Conn.RemoteAddr().String()] = &Client{
					Name:        name,
					Conn:        msg.Conn,
					LastMessage: time.Now(),
				}
				log.Printf("%s connected: %s\n", *name, sensitive(addr.IP.String()))
			} else {
				msg.Conn.Write([]byte(fmt.Sprintf("You are banned buddy: %f seconds left\n", BanLimit-now.Sub(bannedAt).Seconds())))
				msg.Conn.Close()
			}
		case ClientDisconnected:
			addr := msg.Conn.RemoteAddr().(*net.TCPAddr)
			delete(clients, addr.String())
			log.Printf("Client %s disconnected\n", sensitive(addr.String()))
		case NewMessage:
			now := time.Now()
			authorAddr := msg.Conn.RemoteAddr().(*net.TCPAddr)
			author, authorExists := clients[authorAddr.String()]
			if authorExists {
				if now.Sub(author.LastMessage).Seconds() >= MessageRate {
					if utf8.ValidString(msg.Text) {
						author.StrikeCount = 0
						author.LastMessage = now
						payload := fmt.Sprintf("%s: %s", *msg.Name, msg.Text)
						log.Printf("%s sent: %s", *msg.Name, msg.Text)
						for _, client := range clients {
							if client.Conn.RemoteAddr().String() != authorAddr.String() {
								_, err := client.Conn.Write([]byte(payload))
								if err != nil {
									log.Printf("could not send data to %s: %s\n", sensitive(client.Conn.RemoteAddr().String()), sensitive(err.Error()))
								}
							}
						}
					} else {
						author.StrikeCount++
						if author.StrikeCount > StrikeLimit {
							bannedMfs[authorAddr.IP.String()] = now
							author.Conn.Close()
						}
					}
				} else {
					author.StrikeCount++
					if author.StrikeCount > StrikeLimit {
						bannedMfs[authorAddr.IP.String()] = now
						author.Conn.Close()
					}
				}
			} else {
				msg.Conn.Close()
			}
		}
	}
}

func client(conn net.Conn, messages chan Message) {
	conn.Write([]byte("Enter your name: "))
	buf := make([]byte, 20)
	// first message is the name of client
	n, err := conn.Read(buf)
	if err != nil {
		log.Printf("could not read name from %s: %s\n", sensitive(conn.RemoteAddr().String()), sensitive(err.Error()))
		conn.Close()
		return
	}
	rawName := string(buf[:n])
	name := cleanName(rawName)

	messages <- Message{
		Name: &name,
		Type: ClientConnected,
		Conn: conn,
	}

	for {
		buf := make([]byte, 64)
		n, err := conn.Read(buf)
		if err != nil {
			log.Printf("could not read from %s: %s\n", sensitive(conn.RemoteAddr().String()), sensitive(err.Error()))
			conn.Close()
			messages <- Message{
				Type: ClientDisconnected,
				Conn: conn,
			}
			return
		}
		messages <- Message{
			Name: &name,
			Type: NewMessage,
			Text: string(buf[:n]),
			Conn: conn,
		}
	}
}

func main() {
	ln, err := net.Listen("tcp", ":"+Port)
	if err != nil {
		log.Fatalf("ERROR: could not listen to port %v: %s\n", Port, err)
	}
	log.Printf("listening to tcp connection on port %s \n", Port)

	messages := make(chan Message)
	go server(messages)

	for {
		conn, err := ln.Accept()
		if err != nil {
			log.Printf("ERROR: could not accept the connection: %s\n", err)
		}

		go client(conn, messages)
	}

}
