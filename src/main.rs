use log::LevelFilter;
use polling::{Event, Poller};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};

const SOCKET_LISTENER_POLL_KEY: usize = 10;
static CLIENT_KEY_MAKE: AtomicUsize = AtomicUsize::new(1000);

fn main() -> anyhow::Result<()> {
    struct Client {
        socket: TcpStream,
        buff: [u8; 1024],
        len: usize,
    }
    //初始化logger
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .init();
    //初始化poll
    let poller = Poller::new()?;
    //创建socket 监听
    let listener = TcpListener::bind("127.0.0.1:8000")?;
    //设置为非堵塞
    listener.set_nonblocking(true)?;
    //设置socket fd 为read 事件触发 accept
    poller.add(&listener, Event::readable(SOCKET_LISTENER_POLL_KEY))?;
    let mut clients = HashMap::new();
    //用来接收事件
    let mut events = Vec::new();
    loop {
        //清理上次事件
        events.clear();
        //等待事件通知,直到有事件为止
        poller.wait(&mut events, None)?;
        for event in events.iter() {
            if event.key == SOCKET_LISTENER_POLL_KEY {
                //表示可accept
                let (socket, addr) = listener.accept()?;
                log::info!("addr:{} connect", addr);
                //设置socket为非堵塞,并产生一个此socket的key
                socket.set_nonblocking(true)?;
                let client_key = CLIENT_KEY_MAKE.fetch_add(1, Ordering::Release);
                // 异步监听此socket read,并将此socket封装成client放入map中以供事件触发时查找
                poller.add(&socket, Event::readable(client_key))?;
                clients.insert(
                    client_key,
                    Client {
                        socket,
                        buff: [0; 1024],
                        len: 0,
                    },
                );
            } else if let Some(client) = clients.get_mut(&event.key) {
                //如果是client事件 判断事件状态,然后根据read,write 进行相应的处理
                let mut disconnect = false;
                if event.readable {
                    let size = match client.socket.read(&mut client.buff[..]) {
                        Ok(n) => n,
                        Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => 0,
                        Err(err) => {
                            log::error!("addr:{} error:{}", client.socket.peer_addr()?, err);
                            0
                        }
                    };
                    client.len = size;
                    disconnect = size == 0;
                    //读取到数据后设置成等待write以触发写入
                    poller.modify(&client.socket, Event::writable(event.key))?;
                } else if event.writable {
                    if let Err(err) = client.socket.write(&client.buff[..client.len]) {
                        log::error!("addr:{} error:{}", client.socket.peer_addr()?, err);
                        disconnect = true;
                    }
                    //发送后设置成等待read
                    poller.modify(&client.socket, Event::readable(event.key))?;
                }
                if disconnect {
                    let client = clients.remove(&event.key).unwrap();
                    poller.delete(&client.socket)?;
                    log::info!("addr:{} disconnect", client.socket.peer_addr()?);
                }
            }
        }
    }
}
