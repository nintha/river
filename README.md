# River
Pure Rust Implementation of RTMP Live Stream Server

## Usage
```
USAGE:
    river.exe [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --http-flv-port <http-flv-port>    [default: 8080]
        --rtmp-port <rtmp-port>            [default: 1935]
        --ws-fmp4-port <ws-fmp4-port>      [default: 8082]
        --ws-h264-port <ws-h264-port>      [default: 8081]
```
## Push

OBS, x264, tune=zerolatency, CBR, perset=veryfast, porfile=baseline

## Play

### ffplay
```shell
ffplay -fflags nobuffer -analyzeduration 100000 rtmp://localhost:11935/channel/token
```
 `-fflags nobuffer -analyzeduration 100000` could reduce the latency. In my computer, the latency is about 1 second.

### JMuxer
Playing in the browser with [Jmuxer](https://github.com/samirkumardas/jmuxer).

If pushing stream with x264 codec, recommand profile is baseline 

If you are using x264 encoding to push the stream, it is recommended that profile=baseline to avoid frequent video jitter. The current local test latency is about 1 second.

## Completed
- [x] support custom width and height
- [x] support audio
- [x] support HTTP-FLV output
- [x] support raw H264 stream output
- [x] Deal with the problem of websocket message backlog
- [x] Configurable startup parameters (monitoring server port)

## TODO
- [ ] PUSH/PULL authentication
- [ ] support fragmented MP4 output


## FAQ

### The Chrome auto pauses muted video in inactive tabs.

- Listen to the event `visibilitychange`, and change Video playback progress manually

```js
var video = document.getElementById('video');
document.addEventListener("visibilitychange", function() {
  video.currentTime = video.buffered.end(0);
});
```
- Timed changing Video playback progress manually
```js
var video = document.getElementById('video');
setInterval(()=>{
  var latest = video.buffered.end(0);
  // over 200ms
  if (latest - video.currentTime > 0.2){
    video.currentTime = latest;
  }
}, 1000);
```

## Reference
- [RTMP推送AAC ADTS音频流](https://www.jianshu.com/p/1a6f195863c7)
- [视音频数据处理入门](https://blog.csdn.net/leixiaohua1020/article/details/50534369)
- [rtmp数据封装](https://blog.csdn.net/Jacob_job/article/details/81880445)
- [视音频编解码学习工程：FLV封装格式分析器](https://blog.csdn.net/leixiaohua1020/article/details/17934487)

Thanks to [Jetbrains](https://www.jetbrains.com/?from=River) for their great IDEs and the free [open source license](https://jb.gg/OpenSource).

![](doc/jetbrains.webp)
