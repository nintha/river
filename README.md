# River
RTMP Live Stream Server

## Push

OBS

## Play

```shell
ffplay -fflags nobuffer -analyzeduration 100000 rtmp://localhost:11935/channel/token
```

使用`-fflags nobuffer -analyzeduration 100000`可以有效降低播放的延迟，目前本地测试延迟大概为1秒

## Completed
- [x] 支持不同分辨率的推流和拉流（之前默认1028x720）
- [x] 支持音频传输

## TODO
- [ ] 支持HTTP-FLV输出

## 参考资料
- [RTMP推送AAC ADTS音频流](https://www.jianshu.com/p/1a6f195863c7)
- [视音频数据处理入门](https://blog.csdn.net/leixiaohua1020/article/details/50534369)
- [rtmp数据封装](https://blog.csdn.net/Jacob_job/article/details/81880445)
- [视音频编解码学习工程：FLV封装格式分析器](https://blog.csdn.net/leixiaohua1020/article/details/17934487)