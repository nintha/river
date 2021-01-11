# River
Pure Rust Implementation of RTMP Live Stream Server

## Push

OBS, x264, tune=zerolatency, CBR, perset=veryfast, porfile=baseline

## Play

### ffplay
```shell
ffplay -fflags nobuffer -analyzeduration 100000 rtmp://localhost:11935/channel/token
```
使用`-fflags nobuffer -analyzeduration 100000`可以有效降低播放的延迟，目前本地测试延迟大概为1秒

### JMuxer
使用[Jmuxer](https://github.com/samirkumardas/jmuxer)在浏览器中播放，详见`example/h264-nalu-stream`目录。

如果是使用x264编码推流，建议profile=baseline，可以避免视频频繁抖动，目前本地测试延迟大概为1秒

## Completed
- [x] 支持不同分辨率的推流和拉流（之前默认1028x720）
- [x] 支持音频传输
- [x] 支持HTTP-FLV输出
- [x] 输出H264流，使用[Jmuxer](https://github.com/samirkumardas/jmuxer)在浏览器中播放（视频+音频）
- [x] 处理websocket消息积压的问题

## TODO
- [ ] 推流端认证
- [ ] 可配置的启动参数

## FAQ

### 处理Chrome浏览器会暂停非活动标签页中无声视频的问题

- 监听相关事件，手动调整视频播放进度

```js
var video = document.getElementById('video');
document.addEventListener("visibilitychange", function() {
  video.currentTime = video.buffered.end(0);
});
```
- 定时检查已缓冲的视频长度，追踪播放进度
```js
var video = document.getElementById('video');
setInterval(()=>{
  var latest = video.buffered.end(0);
  // 超过200ms
  if (latest - video.currentTime > 0.2){
    video.currentTime = latest;
  }
}, 1000);
```

## 参考资料
- [RTMP推送AAC ADTS音频流](https://www.jianshu.com/p/1a6f195863c7)
- [视音频数据处理入门](https://blog.csdn.net/leixiaohua1020/article/details/50534369)
- [rtmp数据封装](https://blog.csdn.net/Jacob_job/article/details/81880445)
- [视音频编解码学习工程：FLV封装格式分析器](https://blog.csdn.net/leixiaohua1020/article/details/17934487)
