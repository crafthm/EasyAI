import { AppGrid } from './components/AppGrid'
import { Toolbar } from './components/Toolbar'
import { demoApps } from './data/apps'

function App() {
  return (
    <div className="app">
      <Toolbar />
      <AppGrid items={demoApps} />
    </div>
  )
}

export default App