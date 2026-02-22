import { useEffect, useRef, useState } from 'react';
import './RemoteDesktop.css';

interface RemoteDesktopProps {
  params?: {
    serverUrl?: string;
  };
}

interface ServerInfo {
  supports_webrtc: boolean;
  screen_width: number;
  screen_height: number;
}

interface VideoStats {
  fps: number;
  bitrate: number; // kbps
  resolution: string;
  framesDecoded: number;
  packetsLost: number;
}

type ConnectionState = 'connecting' | 'connected' | 'disconnected' | 'error';

export const RemoteDesktopPanel = (props: RemoteDesktopProps) => {
  const videoRef = useRef<HTMLVideoElement>(null);
  const pcRef = useRef<RTCPeerConnection | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>('connecting');
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [sessionId, setSessionId] = useState<string>('');
  const [videoStats, setVideoStats] = useState<VideoStats>({
    fps: 0,
    bitrate: 0,
    resolution: '-',
    framesDecoded: 0,
    packetsLost: 0
  });
  
  // 自动重连相关状态
  const [retryCount, setRetryCount] = useState(0);
  const [isRetrying, setIsRetrying] = useState(false);
  const [blackScreenDetected, setBlackScreenDetected] = useState(false);
  const maxRetries = 3;
  
  const serverUrl = props.params?.serverUrl || `${window.location.protocol}//${window.location.host}`;

  // 自动重连逻辑 - 检测黑屏并自动重试
  useEffect(() => {
    if (connectionState === 'connected' && !isRetrying) {
      // 连接成功后，检查视频是否真的有画面
      const checkVideoHealth = setInterval(() => {
        const video = videoRef.current;
        if (!video) return;
        
        // 检测黑屏：连接成功但视频尺寸为0或帧率为0
        const hasVideo = video.videoWidth > 0 && video.videoHeight > 0;
        const hasFrames = videoStats.framesDecoded > 0;
        const hasBitrate = videoStats.bitrate > 0;
        
        console.log('[Health Check] Video:', hasVideo, 'Frames:', hasFrames, 'Bitrate:', hasBitrate, 
          'Resolution:', video.videoWidth + 'x' + video.videoHeight);
        
        // 如果连接成功但没有视频数据，认为是黑屏
        if (!hasVideo || (!hasFrames && !hasBitrate)) {
          console.warn('[Health Check] Black screen detected! Video connected but no data.');
          setBlackScreenDetected(true);
          
          if (retryCount < maxRetries) {
            console.log(`[Health Check] Retrying connection... (${retryCount + 1}/${maxRetries})`);
            setIsRetrying(true);
            setRetryCount(prev => prev + 1);
            
            // 清理当前连接并重新连接
            cleanup();
            setConnectionState('connecting');
            
            // 延迟后重新初始化 - 使用新的会话
            setTimeout(() => {
              window.location.reload(); // 简单刷新页面重新连接
            }, 1500);
          } else {
            console.error('[Health Check] Max retries reached, giving up.');
            setConnectionState('error');
          }
        } else {
          // 视频正常，重置重试计数
          if (retryCount > 0) {
            console.log('[Health Check] Video is healthy, resetting retry count');
            setRetryCount(0);
            setBlackScreenDetected(false);
            setIsRetrying(false);
          }
        }
      }, 3000); // 每3秒检查一次
      
      return () => clearInterval(checkVideoHealth);
    }
  }, [connectionState, videoStats, retryCount, isRetrying]);

  // 获取服务器信息并创建会话
  useEffect(() => {
    // 防止重复创建会话的标志
    let isCancelled = false;
    let sessionCreated = false;
    
    const initWebRTC = async () => {
      try {
        // 1. 获取服务器信息
        const infoRes = await fetch(`${serverUrl}/api/rdp/info`);
        const info = await infoRes.json();
        if (isCancelled) return;
        
        setServerInfo(info);
        console.log('Server info:', info);

        // 2. 创建会话
        if (sessionCreated) {
          console.log('Session already created, skipping...');
          return;
        }
        sessionCreated = true;
        
        const sessionRes = await fetch(`${serverUrl}/api/rdp/session`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ fps: 30 }),
        });
        const sessionData = await sessionRes.json();
        
        if (isCancelled) return;
        
        if (!sessionData.session_id) {
          throw new Error('Failed to create session');
        }
        
        setSessionId(sessionData.session_id);
        console.log('Session created:', sessionData.session_id);

        // 3. 创建 WebRTC 连接
        await createPeerConnection(sessionData.session_id);
      } catch (error) {
        console.error('Failed to initialize WebRTC:', error);
        if (!isCancelled) {
          setConnectionState('error');
        }
      }
    };

    initWebRTC();

    return () => {
      isCancelled = true;
      cleanup();
    };
  }, [serverUrl]);

  const createPeerConnection = async (sid: string) => {
    try {
      // 创建 RTCPeerConnection
      const pc = new RTCPeerConnection({
        iceServers: [
          { urls: 'stun:stun.l.google.com:19302' },
          { urls: 'stun:stun1.l.google.com:19302' },
        ],
      });
      pcRef.current = pc;
      
      // 暴露到window用于调试
      (window as any).webrtcPC = pc;

      // 处理远程视频流
      let mediaStream: MediaStream | null = null;
      pc.ontrack = (event) => {
        console.log('Received remote track:', event.streams);
        if (videoRef.current && event.streams[0]) {
          mediaStream = event.streams[0];
          videoRef.current.srcObject = mediaStream;
          console.log('Video srcObject set, waiting for connection to play...');
        }
      };

      // 监听连接状态
      pc.onconnectionstatechange = () => {
        console.log('Connection state:', pc.connectionState);
        if (pc.connectionState === 'connected') {
          setConnectionState('connected');
          // 连接成功后播放视频
          if (videoRef.current && videoRef.current.srcObject) {
            console.log('Connection connected, playing video...');
            videoRef.current.play().then(() => {
              console.log('Video playing successfully');
            }).catch(e => {
              console.error('Video play failed:', e);
              // 重试播放
              setTimeout(() => {
                videoRef.current?.play().catch(e2 => console.error('Video play retry failed:', e2));
              }, 500);
            });
          }
          // 开始收集统计信息
          startStatsCollection(pc);
        } else if (pc.connectionState === 'failed' || pc.connectionState === 'closed') {
          setConnectionState('disconnected');
        }
      };

      pc.oniceconnectionstatechange = () => {
        console.log('ICE connection state:', pc.iceConnectionState);
      };

      // 收集 ICE candidate
      const iceCandidates: RTCIceCandidate[] = [];
      pc.onicecandidate = (event) => {
        if (event.candidate) {
          iceCandidates.push(event.candidate);
          console.log('ICE candidate:', event.candidate.candidate);
        }
      };

      // 创建 DataChannel 用于输入控制
      const dataChannel = pc.createDataChannel('input', { ordered: true });
      dataChannel.onopen = () => console.log('DataChannel opened');
      dataChannel.onclose = () => console.log('DataChannel closed');

      // 添加视频接收轨道
      pc.addTransceiver('video', { direction: 'recvonly' });

      // 创建 Offer
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);

      console.log('Created offer:', offer.sdp?.substring(0, 100));

      // 发送 Offer 到服务器
      const offerRes = await fetch(`${serverUrl}/api/rdp/offer`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          sdp: offer.sdp,
          session_id: sid,
        }),
      });

      const answerData = await offerRes.json();
      if (!answerData.success) {
        throw new Error('Failed to get answer from server');
      }

      console.log('Received answer');

      // 设置远程描述
      await pc.setRemoteDescription(new RTCSessionDescription({
        type: 'answer',
        sdp: answerData.sdp,
      }));

      // 发送 ICE candidates
      setTimeout(async () => {
        for (const candidate of iceCandidates) {
          await fetch(`${serverUrl}/api/rdp/ice`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              candidate: candidate.candidate,
              sdpMid: candidate.sdpMid,
              sdpMLineIndex: candidate.sdpMLineIndex,
              session_id: sid,
            }),
          });
        }
      }, 1000);

      // 获取服务器 ICE candidates
      setTimeout(async () => {
        try {
          const candidatesRes = await fetch(`${serverUrl}/api/rdp/ice-candidates`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ session_id: sid }),
          });
          const candidatesData = await candidatesRes.json();
          
          if (candidatesData.success && candidatesData.candidates) {
            for (const cand of candidatesData.candidates) {
              await pc.addIceCandidate(new RTCIceCandidate({
                candidate: cand.candidate,
                sdpMid: cand.sdpMid,
                sdpMLineIndex: cand.sdpMLineIndex,
              }));
            }
          }
        } catch (e) {
          console.error('Failed to get server ICE candidates:', e);
        }
      }, 2000);

    } catch (error) {
      console.error('Failed to create peer connection:', error);
      setConnectionState('error');
    }
  };

  const startStatsCollection = (pc: RTCPeerConnection) => {
    let lastStats: any = null;
    let lastTimestamp = 0;
    
    const collectStats = async () => {
      try {
        const stats = await pc.getStats();
        let currentStats: any = null;
        
        stats.forEach((stat) => {
          if (stat.type === 'inbound-rtp' && stat.kind === 'video') {
            currentStats = stat;
          }
        });
        
        if (currentStats) {
          const now = currentStats.timestamp;
          
          if (lastStats && lastTimestamp > 0) {
            const timeDelta = (now - lastTimestamp) / 1000; // 转换为秒
            
            if (timeDelta > 0) {
              // 计算帧率 (FPS)
              const framesDelta = currentStats.framesDecoded - lastStats.framesDecoded;
              const fps = Math.round(framesDelta / timeDelta);
              
              // 计算码率 (kbps)
              const bytesDelta = currentStats.bytesReceived - lastStats.bytesReceived;
              const bitrate = Math.round((bytesDelta * 8) / timeDelta / 1000);
              
              // 更新状态
              setVideoStats({
                fps: fps,
                bitrate: bitrate,
                resolution: currentStats.frameWidth && currentStats.frameHeight 
                  ? `${currentStats.frameWidth}x${currentStats.frameHeight}`
                  : '-',
                framesDecoded: currentStats.framesDecoded,
                packetsLost: currentStats.packetsLost || 0
              });
            }
          }
          
          lastStats = currentStats;
          lastTimestamp = now;
        }
      } catch (e) {
        console.error('Failed to get stats:', e);
      }
    };
    
    // 每秒收集一次统计信息以计算实时帧率和码率
    const interval = setInterval(collectStats, 1000);
    
    // 立即收集一次
    setTimeout(collectStats, 1000);
    
    // 清理函数
    (window as any).stopStatsCollection = () => clearInterval(interval);
  };

  const cleanup = () => {
    if (pcRef.current) {
      pcRef.current.close();
      pcRef.current = null;
    }
  };

  return (
    <div className="remote-desktop-panel">
      <div className="rdp-toolbar">
        <div className="rdp-toolbar-left">
          <span className={`rdp-status rdp-status-${connectionState}`}>
            {connectionState === 'connecting' && '连接中...'}
            {connectionState === 'connected' && '已连接'}
            {connectionState === 'disconnected' && '已断开'}
            {connectionState === 'error' && '连接错误'}
          </span>
          {serverInfo && (
            <span className="rdp-resolution">
              服务器: {serverInfo.screen_width}x{serverInfo.screen_height}
            </span>
          )}
          {sessionId && (
            <span className="rdp-session-id">
              Session: {sessionId.substring(0, 8)}...
            </span>
          )}
        </div>
        <div className="rdp-toolbar-right">
          {connectionState === 'connected' && (
            <div className="rdp-stats">
              <span className="rdp-stat-item" title="帧率">
                <span className="rdp-stat-label">FPS:</span>
                <span className="rdp-stat-value">{videoStats.fps}</span>
              </span>
              <span className="rdp-stat-item" title="码率">
                <span className="rdp-stat-label">码率:</span>
                <span className="rdp-stat-value">{videoStats.bitrate} kbps</span>
              </span>
              <span className="rdp-stat-item" title="分辨率">
                <span className="rdp-stat-label">分辨率:</span>
                <span className="rdp-stat-value">{videoStats.resolution}</span>
              </span>
              <span className="rdp-stat-item" title="丢包">
                <span className="rdp-stat-label">丢包:</span>
                <span className="rdp-stat-value">{videoStats.packetsLost}</span>
              </span>
            </div>
          )}
        </div>
      </div>
      
      <div className="rdp-video-container">
        <video
          ref={videoRef}
          autoPlay
          playsInline
          muted
          className="rdp-video"
        />
      </div>
      
      <div className="rdp-instructions">
        <p>WebRTC 远程桌面 - 使用 video 元素显示</p>
      </div>
    </div>
  );
};

export default RemoteDesktopPanel;
