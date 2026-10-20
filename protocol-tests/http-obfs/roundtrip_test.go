package compat_test

import (
 "bufio"
 "bytes"
 "crypto/aes"
 "crypto/cipher"
 "crypto/md5"
 "crypto/rand"
 "crypto/sha1"
 "encoding/binary"
 "fmt"
 "io"
 "net"
 "net/http"
 "testing"
 "time"
 "golang.org/x/crypto/hkdf"
)

type independentSS struct { aead cipher.AEAD; nonce []byte }
func newIndependentSS(salt []byte) (*independentSS,error) {
 key:=md5.Sum([]byte("local-test-only")); sub:=make([]byte,16)
 if _,err:=io.ReadFull(hkdf.New(sha1.New,key[:],salt,[]byte("ss-subkey")),sub);err!=nil{return nil,err}
 block,err:=aes.NewCipher(sub);if err!=nil{return nil,err};aead,err:=cipher.NewGCM(block);if err!=nil{return nil,err}
 return &independentSS{aead:aead,nonce:make([]byte,aead.NonceSize())},nil
}
func(s *independentSS) next(){for i:=range s.nonce{s.nonce[i]++;if s.nonce[i]!=0{break}}}
func(s *independentSS) read(r io.Reader)([]byte,error){
 length:=make([]byte,2+s.aead.Overhead());if _,err:=io.ReadFull(r,length);err!=nil{return nil,err}
 plain,err:=s.aead.Open(nil,s.nonce,length,nil);if err!=nil{return nil,err};s.next()
 size:=int(binary.BigEndian.Uint16(plain));if size>0x3fff{return nil,fmt.Errorf("invalid SS length")}
 body:=make([]byte,size+s.aead.Overhead());if _,err=io.ReadFull(r,body);err!=nil{return nil,err}
 plain,err=s.aead.Open(nil,s.nonce,body,nil);s.next();return plain,err
}
func(s *independentSS) seal(body []byte)[]byte{
 length:=[]byte{byte(len(body)>>8),byte(len(body))}; wire:=s.aead.Seal(nil,s.nonce,length,nil);s.next();wire=append(wire,s.aead.Seal(nil,s.nonce,body,nil)...);s.next();return wire
}

// This is an independent HTTP-obfs + AES-GCM session server. The server
// decrypts destination and request, then encrypts a real HTTP response. It
// intentionally fragments the envelope across writes; no upstream cipher or
// obfs server implementation is reused to accept a matching client mistake.
func TestRoundTrip(t *testing.T){
 for _,client:=range clients {t.Run(client.name,func(t *testing.T){
  raw,server:=net.Pipe();defer raw.Close();defer server.Close();deadline:=time.Now().Add(3*time.Second);raw.SetDeadline(deadline);server.SetDeadline(deadline)
  response:=[]byte("HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nchecked")
  request:=[]byte("GET /acceptance HTTP/1.1\r\nHost: target.example\r\n\r\n")
  result:=make(chan error,1)
  go func(){result<-func()error{
   reader:=bufio.NewReader(server);req,err:=http.ReadRequest(reader);if err!=nil{return err}
   if req.Method!="GET"||req.URL.Path!="/"||req.Host!="cover.example:443"||req.Header.Get("Upgrade")!="websocket"{return fmt.Errorf("invalid envelope")}
   first,err:=io.ReadAll(req.Body);if err!=nil{return err};body:=io.MultiReader(bytes.NewReader(first),reader)
   salt:=make([]byte,16);if _,err=io.ReadFull(body,salt);err!=nil{return err};ss,err:=newIndependentSS(salt);if err!=nil{return err}
   plain,err:=ss.read(body);if err!=nil{return err};destination:=append([]byte{3,14},[]byte("target.example")...);destination=append(destination,1,187)
   if !bytes.HasPrefix(plain,destination){return fmt.Errorf("wrong destination")}
   payload:=plain[len(destination):];for len(payload)<len(request){chunk,e:=ss.read(body);if e!=nil{return e};payload=append(payload,chunk...)}
   if !bytes.Equal(payload,request){return fmt.Errorf("request failed independent authentication")}
   responseSalt:=make([]byte,16);if _,err=rand.Read(responseSalt);err!=nil{return err};reply,err:=newIndependentSS(responseSalt);if err!=nil{return err}
   for _,part:=range [][]byte{[]byte("HTTP/1.1 101 Switching"),[]byte(" Protocols\r\nUpgrade: websocket\r\n\r\n"),responseSalt,reply.seal(response)}{if _,err=server.Write(part);err!=nil{return err}}
   return nil
  }()}()
  stream:=client.encrypted(t,client.obfs(raw));if _,err:=stream.Write(request);err!=nil{t.Fatal(err)}
  got:=make([]byte,len(response));if _,err:=io.ReadFull(stream,got);err!=nil{t.Fatal(err)};if !bytes.Equal(got,response){t.Fatal("wrong HTTP response")}
  if err:=<-result;err!=nil{t.Fatal(err)}
 })}
}
